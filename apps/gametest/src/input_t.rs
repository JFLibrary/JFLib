// input_t -- the group "input": names and bindings, the map, the settings file (base::Ini and
// base::Settings), the pointer lock's click and end, frames against ticks, the wheel, capture, plat's
// events, the factor per platform; (G7a part 2) game pads: their sources, the radial dead zone, analog
// axes, looking at a turn rate with a curve, pad play without the lock, which pad drives the input, the
// pad settings in the file; every case against the rule of docs/GAMECORE.md written out

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::{Ini, Schema, Settings};
use game::input::{self, MOUSE_LEFT, MOUSE_RIGHT, MOUSE_X2, PAD_DIR_PRESS, PAD_DIR_RELEASE, TURN_RATE, platform_motion_factor};
use game::{ActionId, ActionKind, ActionMap, Bind, Device, Input, MouseSettings, PadSettings, Source};
use plat::{Gamepad, KC_A, KC_D, KC_DOWN, KC_Q, KC_S, KC_SPACE, KC_UP, KC_W, PAD_A, PAD_B, PAD_HOME, PAD_LX, PAD_LY, PAD_RT, PAD_RX, PAD_RY, PAD_X, PadStyle, PointerLock};

use crate::Ctx;

pub fn run(c: &mut Ctx) {
    names(c);
    map(c);
    settings(c);
    lock(c);
    frames_and_ticks(c);
    wheel_buttons_capture(c);
    plat_events(c);
    factor(c);
    pad_names(c);
    pad_sticks(c);
    pad_play(c);
    pad_axes_and_look(c);
    pad_which(c);
    pad_settings(c);
}

/// the actions of the tests
struct A {
    fwd: ActionId,
    side: ActionId,
    fire: ActionId,
    jump: ActionId,
    next: ActionId,
    look_x: ActionId,
    look_y: ActionId,
    zoom: ActionId,
}

fn actions() -> (ActionMap, A) {
    let mut m = ActionMap::new();
    let a = A {
        fwd: m.add("forward", ActionKind::Axis, &[Bind::key(KC_W), Bind::scaled(Source::Key(KC_S), -1.0)]),
        side: m.add("strafe", ActionKind::Axis, &[Bind::key(KC_D), Bind::scaled(Source::Key(KC_A), -1.0)]),
        fire: m.add("fire", ActionKind::Button, &[Bind::new(Source::Mouse(MOUSE_LEFT))]),
        jump: m.add("jump", ActionKind::Button, &[Bind::key(KC_SPACE)]),
        next: m.add("next", ActionKind::Button, &[Bind::new(Source::WheelUp)]),
        look_x: m.add("look_x", ActionKind::Motion, &[Bind::new(Source::MouseX)]),
        look_y: m.add("look_y", ActionKind::Motion, &[Bind::new(Source::MouseY)]),
        zoom: m.add("zoom", ActionKind::Motion, &[Bind::new(Source::Wheel)]),
    };
    (m, a)
}

/// an input that plays without a lock (a platform without one)
fn playing() -> (Input, A) {
    let (m, a) = actions();
    let mut i = Input::new(m);
    i.set_lock(PointerLock::Unsupported);
    (i, a)
}

/// radians per raw unit at sensitivity s and the platform's factor
fn k(s: f64) -> f64 {
    platform_motion_factor() * s * input::DEGREES_PER_COUNT * (core::f64::consts::PI / 180.0)
}

fn names(c: &mut Ctx) {
    let (mut n, mut bad) = (0, 0);
    for (u, _, _, _, name) in plat::key_code_rows_test() {
        let s = Source::Key(u);
        n += 1;
        if (!name.is_empty() && s.name() != name) || Source::parse(&s.name()) != Some(s) || s.label().is_empty() {
            bad += 1;
        }
    }
    c.check(bad == 0 && n > 100, || format!("keys by name: {bad} of {n} wrong"));
    for b in 1..=7u8 {
        let s = Source::Mouse(b);
        c.check(Source::parse(&s.name()) == Some(s) && s.name() == format!("Mouse{b}"), || format!("Mouse{b}"));
    }
    for s in [Source::WheelUp, Source::WheelDown, Source::MouseX, Source::MouseY, Source::Wheel] {
        c.check(Source::parse(&s.name()) == Some(s), || format!("{s:?}"));
    }
    c.check(Source::Key(KC_W).name() == "KeyW" && Source::parse("KeyW") == Some(Source::Key(KC_W)), || "KeyW".into());
    c.check(Source::parse("Mouse0").is_none() && Source::parse("Mouse8").is_none() && Source::parse("Bogus").is_none() && Source::parse("").is_none(),
            || "unknown names".into());
    c.check(Source::parse("Code0x00").is_none() && Source::parse("Code0x100").is_none(), || "Code0x out of range".into());
    c.check(Source::Mouse(MOUSE_LEFT).label() == "Linke Maustaste" && Source::Mouse(MOUSE_X2).label() == "Maustaste 5", || "mouse labels".into());
    c.check(Source::MouseX.is_motion() && Source::Wheel.is_motion() && !Source::WheelUp.is_motion() && !Source::Key(KC_W).is_motion(), || "is_motion".into());
    // bindings' texts
    for (b, t) in [(Bind::key(KC_D), "KeyD"), (Bind::scaled(Source::Key(KC_A), -1.0), "-KeyA"), (Bind::scaled(Source::MouseY, 0.5), "MouseY*0.5"),
                   (Bind::scaled(Source::MouseX, -2.25), "MouseX*-2.25"), (Bind::new(Source::Mouse(MOUSE_RIGHT)), "Mouse2")] {
        c.check(b.text() == t && Bind::parse(t) == Some(b), || format!("{t}: {} / {:?}", b.text(), Bind::parse(t)));
    }
    c.check(Bind::parse("-MouseY*0.5") == Some(Bind::scaled(Source::MouseY, -0.5)), || "-NAME*SCALE".into());
    c.check(Bind::parse("KeyW*").is_none() && Bind::parse("KeyW*x").is_none() && Bind::parse("Nope").is_none(), || "bad texts".into());
}

fn map(c: &mut Ctx) {
    let (mut m, a) = actions();
    c.check(m.len() == 8 && m.find("fire") == Some(a.fire) && m.find("nope").is_none() && m.name(a.zoom) == "zoom", || "find, name".into());
    c.check(m.kind(a.look_x) == ActionKind::Motion && m.binds(a.fwd).len() == 2, || "kind, binds".into());
    // a source the kind does not take, a source twice
    c.check(!m.bind(a.fire, Bind::new(Source::MouseX)) && !m.bind(a.look_x, Bind::key(KC_Q)) && !m.bind(a.jump, Bind::key(KC_SPACE)), || "bind refuses".into());
    c.check(m.bind(a.jump, Bind::key(KC_Q)) && m.binds(a.jump).len() == 2 && m.users(Source::Key(KC_Q)) == [a.jump], || "bind, users".into());
    c.check(m.unbind(a.jump, Source::Key(KC_Q)) && !m.unbind(a.jump, Source::Key(KC_Q)), || "unbind".into());
    c.check(!m.set_binds(a.fwd, &[Bind::key(KC_UP), Bind::key(KC_UP)]) && !m.set_binds(a.fwd, &[Bind::new(Source::MouseY)]) && m.binds(a.fwd).len() == 2,
            || "set_binds refuses twice and the wrong kind".into());
    c.check(m.set_binds(a.fwd, &[Bind::key(KC_UP)]) && m.binds(a.fwd) == [Bind::key(KC_UP)], || "set_binds".into());
    m.reset(a.fwd);
    c.check(m.binds(a.fwd) == m.defaults(a.fwd), || "reset".into());
    let mut m2 = ActionMap::new();
    let x = m2.add("mixed", ActionKind::Button, &[Bind::key(KC_W), Bind::new(Source::MouseX), Bind::key(KC_W)]);
    c.check(m2.binds(x) == [Bind::key(KC_W)], || "add leaves out what the kind does not take and doubles".into());
    c.check(m.users(Source::Key(KC_W)) == [a.fwd] && m.ids().count() == 8, || "users, ids".into());
}

fn same_binds(a: &ActionMap, b: &ActionMap) -> bool {
    a.len() == b.len() && a.ids().all(|i| a.binds(i) == b.binds(i))
}

static SCHEMA: Schema = Schema::new(1, &[]);

fn settings(c: &mut Ctx) {
    let (mut m, a) = actions();
    let mut ini = Ini::new();
    m.store(&mut ini, "input");
    c.check(ini.section("input").is_empty(), || "defaults store nothing".into());
    m.set_binds(a.fwd, &[Bind::key(KC_UP), Bind::scaled(Source::Key(KC_DOWN), -1.0)]);
    m.set_binds(a.jump, &[]);
    m.set_binds(a.look_y, &[Bind::scaled(Source::MouseY, -0.5)]);
    m.store(&mut ini, "input");
    c.check(ini.get("input", "bind.forward") == Some("ArrowUp -ArrowDown") && ini.get("input", "bind.jump") == Some("none")
            && ini.get("input", "bind.look_y") == Some("MouseY*-0.5") && ini.get("input", "bind.fire").is_none(),
            || format!("stored: {:?}", ini.section("input")));
    // through the file's text and back
    let text = ini.to_bytes();
    let back = Ini::parse(&text);
    let (mut m2, _) = actions();
    let bad = m2.load(&back, "input");
    c.check(bad == 0 && same_binds(&m, &m2), || format!("loaded: {bad} bad"));
    // the defaults again: their lines go
    m.reset_all();
    m.store(&mut ini, "input");
    c.check(ini.section("input").is_empty(), || "a reset removes the lines".into());
    // what cannot be read is left out
    ini.set("input", "bind.jump", "Space Bogus Mouse9 MouseX Space");
    let (mut m3, a3) = actions();
    let bad = m3.load(&ini, "input");
    c.check(bad == 4 && m3.binds(a3.jump) == [Bind::key(KC_SPACE)], || format!("unreadable: {bad}, {:?}", m3.binds(a3.jump)));
    // mouse settings
    let mut ini = Ini::new();
    MouseSettings::DEFAULT.store(&mut ini, "input");
    c.check(ini.section("input").is_empty() && MouseSettings::load(&ini, "input") == MouseSettings::DEFAULT, || "default mouse settings".into());
    let s = MouseSettings { sensitivity: 2.5, invert_x: false, invert_y: true, factor: Some(1.7) };
    s.store(&mut ini, "input");
    let back = Ini::parse(&ini.to_bytes());
    c.check(MouseSettings::load(&back, "input") == s, || format!("mouse settings: {:?}", ini.section("input")));
    ini.set("input", "sensitivity", "1e9");
    ini.set("input", "motion_factor", "abc");
    let l = MouseSettings::load(&ini, "input");
    c.check(l.sensitivity == 100.0 && l.factor.is_none() && l.invert_y, || format!("clamped, unreadable: {l:?}"));
    // the library's settings: the free-form part of a base::Settings file (mem://, no real file)
    let path = "mem://gametest/prefs.ini";
    plat::write_file(path, b"");
    let (m, a) = actions();
    let mut input = Input::new(m);
    input.edit_map(|m| m.set_binds(a.side, &[Bind::key(KC_Q), Bind::scaled(Source::Key(KC_A), -1.0)]));
    input.set_mouse(MouseSettings { sensitivity: 0.75, invert_x: true, invert_y: false, factor: None });
    let mut st = Settings::open(path, &SCHEMA);
    input.store(st.ini_mut(), "input");
    c.check(st.save(), || "Settings::save".into());
    let st2 = Settings::open(path, &SCHEMA);
    let (m, _) = actions();
    let mut input2 = Input::new(m);
    let bad = input2.load(st2.ini(), "input");
    c.check(bad == 0 && same_binds(input.map(), input2.map()) && input2.mouse() == input.mouse(), || "through base::Settings and back".into());
    let file = plat::read_file(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    c.check(file.contains("bind.strafe=KeyQ -KeyA") && file.contains("sensitivity=0.75") && file.contains("invert_x=true"), || format!("the file: {file:?}"));
}

/// the pointer lock: the click that asks does not fire, Esc's end of the lock pauses and lets go
fn lock(c: &mut Ctx) {
    let (m, a) = actions();
    let mut i = Input::new(m);
    c.check(!i.playing() && i.require_lock() && i.lock() == PointerLock::Off, || "not playing before the lock".into());
    i.key(KC_W, true);
    c.check(!i.down(a.fwd) && i.axis(a.fwd) == 0.0 && i.key_held(KC_W), || "a key before the lock drives nothing (it is held)".into());
    // the click that asks for the lock
    i.button(MOUSE_LEFT, true);
    c.check(i.take_lock_request() && !i.take_lock_request(), || "a press asks for the lock once".into());
    c.check(!i.pressed(a.fire) && !i.down(a.fire), || "the lock's click fires nothing".into());
    i.set_lock(PointerLock::Locked);
    c.check(i.playing() && i.down(a.fwd) && !i.pressed(a.fwd) && i.axis(a.fwd) == 1.0, || "W held from before counts, without a press".into());
    i.button(MOUSE_LEFT, false);
    c.check(!i.released(a.fire) && !i.down(a.fire), || "the lock click's release is the lock's too".into());
    let t = i.tick();
    c.check(!t.pressed(a.fire) && !t.released(a.fire) && !t.down(a.fire) && t.down(a.fwd) && t.value(a.fwd) == 1.0, || "the tick saw no fire".into());
    // playing: a click fires
    i.button(MOUSE_LEFT, true);
    c.check(i.pressed(a.fire) && i.down(a.fire) && !i.take_lock_request(), || "a click while locked fires".into());
    i.motion(100.0, -50.0);
    c.check(i.motion_of(a.look_x) == 100.0 * k(1.0) && i.motion_of(a.look_y) == -50.0 * k(1.0), || format!("motion: {} {}", i.motion_of(a.look_x), i.motion_of(a.look_y)));
    let t = i.tick();
    c.check(t.pressed(a.fire) && t.down(a.fire) && t.value(a.look_x) == 100.0 * k(1.0), || "the tick: fire pressed, the motion".into());
    i.end_frame();
    // Esc: the lock ends -> pause, everything let go
    i.set_lock(PointerLock::Off);
    c.check(!i.playing() && i.take_pause() && !i.take_pause() && i.pauses() == 1, || "the end of the lock pauses once".into());
    c.check(i.released(a.fire) && i.released(a.fwd) && !i.down(a.fire) && !i.down(a.fwd) && !i.key_held(KC_W), || "the pause lets everything go".into());
    let t = i.tick();
    c.check(t.released(a.fire) && !t.down(a.fire) && t.released(a.fwd), || "the tick sees the release".into());
    i.motion(10.0, 10.0);
    i.key(KC_SPACE, true);
    c.check(i.motion_of(a.look_x) == 0.0 && !i.pressed(a.jump), || "paused: no motion, no action".into());
    i.end_frame();
    // the browser: the request waits for the next press (Pending); that press is swallowed too
    i.button(MOUSE_LEFT, true);
    c.check(i.take_lock_request(), || "a press while paused asks again".into());
    i.set_lock(PointerLock::Pending);
    i.button(MOUSE_LEFT, false);
    i.button(MOUSE_LEFT, true);
    c.check(!i.playing() && i.take_lock_request() && !i.pressed(a.fire), || "Pending: still swallowed, asked again".into());
    i.set_lock(PointerLock::Locked);
    i.button(MOUSE_LEFT, false);
    c.check(i.playing() && i.down(a.jump) && !i.pressed(a.jump) && !i.released(a.fire) && !i.down(a.fire), || "back: Space held while paused counts, the click's release swallowed".into());
    // Refused: like Off
    i.set_lock(PointerLock::Refused);
    c.check(!i.playing() && i.take_pause() && i.pauses() == 2, || "Refused ends playing".into());
    // a platform without a lock plays, its clicks fire
    let (mut j, aj) = playing();
    j.button(MOUSE_LEFT, true);
    c.check(j.playing() && j.pressed(aj.fire) && !j.take_lock_request(), || "Unsupported: clicks fire".into());
    // a game without the lock
    let (m, a2) = actions();
    let mut g = Input::new(m);
    g.set_require_lock(false);
    g.button(MOUSE_LEFT, true);
    g.set_lock(PointerLock::Off);
    c.check(g.playing() && g.pressed(a2.fire) && !g.take_lock_request() && !g.take_pause(), || "require_lock false".into());
}

fn frames_and_ticks(c: &mut Ctx) {
    let (mut i, a) = playing();
    // a tap between two ticks: one tick with down, pressed, released; the next with nothing
    i.key(KC_SPACE, true);
    i.key(KC_SPACE, false);
    let t = i.tick();
    c.check(t.down(a.jump) && t.pressed(a.jump) && t.released(a.jump) && t.value(a.jump) == 1.0, || format!("tap: {:?}", t.get(a.jump)));
    let t = i.tick();
    c.check(!t.down(a.jump) && !t.pressed(a.jump) && !t.released(a.jump), || "the tick after the tap".into());
    // held over three ticks
    i.key(KC_SPACE, true);
    let t1 = i.tick();
    let t2 = i.tick();
    i.key(KC_SPACE, false);
    let t3 = i.tick();
    c.check(t1.pressed(a.jump) && t1.down(a.jump) && !t2.pressed(a.jump) && t2.down(a.jump) && t3.released(a.jump) && !t3.down(a.jump),
            || "held: pressed once, down, released".into());
    // a frame's presses go with end_frame, the hold stays; a repeat changes nothing
    i.key(KC_SPACE, true);
    i.key(KC_SPACE, true);
    c.check(i.pressed(a.jump) && i.down(a.jump), || "frame: pressed".into());
    i.end_frame();
    c.check(!i.pressed(a.jump) && i.down(a.jump), || "end_frame clears the press".into());
    let t = i.tick();
    c.check(t.pressed(a.jump) && t.get(a.jump) == game::ActionState { down: true, pressed: true, released: false, value: 1.0 }, || "the tick keeps it".into());
    i.key(KC_SPACE, false);
    i.tick();
    // motion: two frames, one tick
    i.motion(10.0, 0.0);
    i.end_frame();
    i.motion(5.0, 0.0);
    c.check(i.motion_of(a.look_x) == 5.0 * k(1.0), || "frame motion".into());
    let t = i.tick();
    c.check(t.value(a.look_x) == 10.0 * k(1.0) + 5.0 * k(1.0) && !t.down(a.look_x), || "tick motion: both frames".into());
    c.check(i.tick().value(a.look_x) == 0.0, || "taken".into());
    // axes: opposite keys cancel, scales, clamp
    i.key(KC_W, true);
    i.key(KC_S, true);
    c.check(i.axis(a.fwd) == 0.0 && i.down(a.fwd), || "W and S cancel".into());
    i.key(KC_S, false);
    i.key(KC_A, true);
    c.check(i.axis(a.fwd) == 1.0 && i.axis(a.side) == -1.0, || "W, A".into());
    i.edit_map(|m| m.bind(a.fwd, Bind::scaled(Source::Key(KC_UP), 0.5)));
    i.key(KC_W, false);
    i.key(KC_UP, true);
    c.check(i.axis(a.fwd) == 0.5, || format!("a scale of 0.5: {}", i.axis(a.fwd)));
    i.key(KC_W, true);
    c.check(i.axis(a.fwd) == 1.0, || "kept at 1".into());
    // a binding that goes while held releases the action
    i.end_frame();
    i.edit_map(|m| m.set_binds(a.side, &[Bind::key(KC_D)]));
    c.check(!i.down(a.side) && i.released(a.side), || "a binding gone while held releases".into());
    // sensitivity and inversion
    let (mut j, aj) = playing();
    j.set_mouse(MouseSettings { sensitivity: 2.0, invert_x: true, invert_y: true, factor: None });
    j.motion(3.0, 4.0);
    c.check(j.motion_of(aj.look_x) == 3.0 * -k(2.0) && j.motion_of(aj.look_y) == 4.0 * -k(2.0), || "sensitivity 2, inverted".into());
    let _ = aj.zoom;
}

fn wheel_buttons_capture(c: &mut Ctx) {
    let (mut i, a) = playing();
    i.wheel(1.0);
    let t = i.tick();
    c.check(t.pressed(a.next) && t.released(a.next) && t.value(a.zoom) == 1.0, || "a notch: a press of WheelUp, Wheel motion 1".into());
    i.wheel(0.5);
    c.check(!i.tick().pressed(a.next), || "half a notch: no press yet".into());
    i.wheel(0.5);
    let t = i.tick();
    c.check(t.pressed(a.next) && t.value(a.zoom) == 0.5, || "the second half".into());
    i.wheel(-1.0);
    let t = i.tick();
    c.check(!t.pressed(a.next) && t.value(a.zoom) == -1.0, || "down: WheelDown is not bound".into());
    // ui's masks: a button not held with both bits went down and up, one held went up and down
    let bit = 1u32 << MOUSE_LEFT;
    i.buttons(bit, 0);
    c.check(i.down(a.fire) && i.pressed(a.fire), || "pressed".into());
    i.end_frame();
    i.buttons(bit, bit);
    c.check(i.down(a.fire) && i.pressed(a.fire) && i.released(a.fire), || "held, up and down in one frame: still held".into());
    i.end_frame();
    i.buttons(0, bit);
    c.check(!i.down(a.fire) && i.released(a.fire), || "released".into());
    i.end_frame();
    i.buttons(bit, bit);
    c.check(!i.down(a.fire) && i.pressed(a.fire) && i.released(a.fire), || "not held, down and up in one frame".into());
    // capture for a settings page: the press answers, drives nothing
    i.end_frame();
    i.start_capture();
    c.check(i.capturing() && i.take_capture().is_none(), || "capturing".into());
    i.key(KC_SPACE, true);
    c.check(i.take_capture() == Some(Source::Key(KC_SPACE)) && !i.capturing() && !i.pressed(a.jump), || "captured a key".into());
    i.start_capture();
    i.button(MOUSE_RIGHT, true);
    c.check(i.take_capture() == Some(Source::Mouse(MOUSE_RIGHT)), || "captured a button".into());
    i.start_capture();
    i.wheel(-1.0);
    c.check(i.take_capture() == Some(Source::WheelDown), || "captured the wheel".into());
}

fn ev(typ: u8, btn: u8, x: i32, data: u32) -> plat::Event {
    plat::Event { typ, btn, mods: 0, win: 200, x, y: 0, data }
}

/// plat's events for a game with its own window
fn plat_events(c: &mut Ctx) {
    let (m, a) = actions();
    let mut i = Input::new(m);
    c.check(i.plat_event(&ev(plat::E_POINTER_LOCK, 0, 0, 1)) && i.lock() == PointerLock::Locked && i.playing(), || "E_POINTER_LOCK 1: locked".into());
    let sc = plat::key_scancode(KC_W) as i32;
    i.plat_event(&ev(plat::E_KEY_DOWN, 0, sc, 0));
    c.check(sc != 0 && i.key_held(KC_W) && i.axis(a.fwd) == 1.0, || format!("E_KEY_DOWN scancode {sc}: KC_W"));
    i.plat_event(&ev(plat::E_DOWN, MOUSE_LEFT, 0, 0));
    c.check(i.pressed(a.fire), || "E_DOWN".into());
    i.plat_event(&ev(plat::E_UP, MOUSE_LEFT, 0, 0));
    c.check(i.released(a.fire), || "E_UP".into());
    i.plat_event(&ev(plat::E_WHEEL, 0, 0, 120));
    c.check(i.tick().pressed(a.next), || "E_WHEEL 120: a notch up".into());
    i.plat_event(&ev(plat::E_KEY_UP, 0, sc, 0));
    c.check(!i.key_held(KC_W), || "E_KEY_UP".into());
    i.plat_event(&ev(plat::E_KEY_DOWN, 0, sc, 0));
    i.plat_event(&ev(plat::E_FOCUS_OUT, 0, 0, 0));
    c.check(!i.key_held(KC_W), || "E_FOCUS_OUT lets go".into());
    // the window holds no lock in plat: data 0 is Off, the pause
    c.check(i.plat_event(&ev(plat::E_POINTER_LOCK, 0, 0, 0)) && !i.playing() && i.take_pause(), || "E_POINTER_LOCK 0: the pause".into());
    c.check(!i.plat_event(&ev(plat::E_PAINT, 0, 0, 0)), || "other events are not the input's".into());
}

fn factor(c: &mut Ctx) {
    let want = if cfg!(windows) {
        input::FACTOR_WINDOWS
    } else if cfg!(target_os = "macos") {
        input::FACTOR_MACOS
    } else if cfg!(target_os = "linux") {
        input::FACTOR_LINUX
    } else {
        input::FACTOR_OTHER
    };
    c.check(platform_motion_factor() == want && MouseSettings::DEFAULT.motion_factor() == want, || format!("this platform's factor {}", platform_motion_factor()));
    let s = MouseSettings { factor: Some(2.0), ..MouseSettings::DEFAULT };
    let (kx, ky) = s.radians_per_unit();
    let e = 2.0 * input::DEGREES_PER_COUNT * (core::f64::consts::PI / 180.0);
    c.check(kx == e && ky == e && s.motion_factor() == 2.0, || "an own factor".into());
    // 1000 counts of the reference mouse at sensitivity 1: 22 degrees
    let (kx, _) = MouseSettings { factor: Some(1.0), ..MouseSettings::DEFAULT }.radians_per_unit();
    let deg = 1000.0 * kx * 180.0 / core::f64::consts::PI;
    c.check((deg - 22.0).abs() < 1e-12, || format!("1000 counts: {deg} degrees"));
    let v: Vec<f64> = [input::FACTOR_WINDOWS, input::FACTOR_LINUX, input::FACTOR_MACOS, input::FACTOR_BROWSER, input::FACTOR_OTHER].to_vec();
    c.check(v.iter().all(|f| *f > 0.0), || "factors positive".into());
}

// ------------------------------------------------------------------ game pads (G7a part 2)

/// a connected pad: buttons held (their value 1), the edges since the last frame, the axes
fn gp(i: u8, pressed: u32, down: u32, up: u32, axes: [f32; 4]) -> Gamepad {
    let mut g = Gamepad::empty(i);
    g.connected = true;
    g.pressed = pressed;
    g.down = down;
    g.up = up;
    g.axes = axes;
    for b in 0..plat::PAD_BUTTONS {
        if pressed & (1 << b) != 0 {
            g.buttons[b] = 1.0;
        }
    }
    g
}

const A: u32 = 1 << PAD_A;
const B: u32 = 1 << PAD_B;
const RT: u32 = 1 << PAD_RT;
const REST: [f32; 4] = [0.0; 4];

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * (1.0 + b.abs())
}

/// a pad's value as the input sees it: the f32 of the snapshot
fn f(v: f32) -> f64 {
    v as f64
}

/// a stick's part beyond the default dead zone of 0.15, scaled to 0 ..= 1
fn beyond(v: f32) -> f64 {
    (f(v) - 0.15) / 0.85
}

fn pad_names(c: &mut Ctx) {
    let mut bad = Vec::new();
    for b in 0..plat::PAD_BUTTONS as u8 {
        let s = Source::PadButton(b);
        if s.name() != format!("Pad{}", plat::pad_button_name(b as usize)) || Source::parse(&s.name()) != Some(s) || !s.is_pad() || s.is_motion() {
            bad.push(s.name());
        }
    }
    for a in 0..4u8 {
        for s in [Source::PadAxis(a), Source::PadDir(a, true), Source::PadDir(a, false)] {
            if Source::parse(&s.name()) != Some(s) || !s.is_pad() || s.is_motion() {
                bad.push(s.name());
            }
        }
    }
    c.check(bad.is_empty(), || format!("pad sources by name: {bad:?}"));
    c.check(Source::PadButton(PAD_A as u8).name() == "PadA" && Source::PadButton(PAD_HOME as u8).name() == "PadHome" && Source::PadAxis(PAD_RY as u8).name() == "PadRightY"
            && Source::PadDir(PAD_LX as u8, false).name() == "PadLeftX-" && Source::PadDir(PAD_LY as u8, true).name() == "PadLeftY+", || "pad names".into());
    c.check(["Pad", "PadZ", "PadLeftX*", "PadLeftXx", "PadLeftZ", "Pada", "PadLeftX+-"].iter().all(|n| Source::parse(n).is_none()), || "unknown pad names".into());
    for (b, t) in [(Bind::scaled(Source::PadAxis(PAD_LY as u8), -1.0), "-PadLeftY"), (Bind::scaled(Source::PadButton(PAD_RT as u8), 0.5), "PadRT*0.5"),
                   (Bind::new(Source::PadDir(PAD_LX as u8, false)), "PadLeftX-"), (Bind::scaled(Source::PadDir(PAD_RX as u8, false), -1.0), "-PadRightX-")] {
        c.check(b.text() == t && Bind::parse(t) == Some(b), || format!("{t}: {} / {:?}", b.text(), Bind::parse(t)));
    }
    let a = Source::PadButton(PAD_A as u8);
    c.check(a.label() == "A" && a.label_for(PadStyle::PlayStation) == "Kreuz" && a.label_for(PadStyle::Nintendo) == "B" && a.label_for(PadStyle::Other) == "A",
            || "the bottom button by style".into());
    c.check(Source::PadButton(PAD_X as u8).label_for(PadStyle::Nintendo) == "Y" && Source::PadButton(PAD_RT as u8).label_for(PadStyle::PlayStation) == "R2"
            && Source::PadAxis(PAD_LX as u8).label() == "Linker Stick waagrecht" && Source::PadDir(PAD_RY as u8, false).label() == "Rechter Stick hoch",
            || "pad labels".into());
    c.check(Source::Key(KC_W).label_for(PadStyle::PlayStation) == Source::Key(KC_W).label(), || "keys: label_for as label".into());
    // which kind takes what
    let (bt, ax, dir) = (Source::PadButton(0), Source::PadAxis(0), Source::PadDir(0, true));
    c.check(ActionKind::Button.takes(bt) && ActionKind::Button.takes(dir) && !ActionKind::Button.takes(ax), || "a Button: pad buttons and directions, no axis".into());
    c.check(ActionKind::Axis.takes(bt) && ActionKind::Axis.takes(dir) && ActionKind::Axis.takes(ax), || "an Axis: all three".into());
    c.check(ActionKind::Motion.takes(ax) && !ActionKind::Motion.takes(bt) && !ActionKind::Motion.takes(dir) && ActionKind::Motion.takes(Source::MouseX), || "a Motion: the axis".into());
}

fn pad_sticks(c: &mut Ctx) {
    let s = PadSettings::DEFAULT;
    c.check(s.dead_zone == 0.15 && s.curve == 2.0 && s.sensitivity == 1.0 && !s.invert_x && !s.invert_y, || "the pad's defaults".into());
    c.check(s.stick(0.1, 0.1) == (0.0, 0.0) && s.stick(0.15, 0.0) == (0.0, 0.0) && s.stick(0.0, 0.0) == (0.0, 0.0), || "inside the dead zone: rest".into());
    // radial: (0.1, 0.1) is 0.141 from the middle, inside; (0.12, 0.12) 0.170, outside, in both axes
    let (x, y) = s.stick(0.12, 0.12);
    c.check(x > 0.0 && x == y, || format!("radial: both axes come together {x} {y}"));
    let (x, y) = s.stick(0.575, 0.0);
    c.check(close(x, 0.5) && y == 0.0, || format!("0.575 with a dead zone of 0.15: 0.5 ({x})"));
    let (x, y) = s.stick(0.6, -0.8);
    c.check(close(x, 0.6) && close(y, -0.8), || format!("at the edge (length 1): unchanged {x} {y}"));
    let (x, y) = s.stick(1.0, 1.0);
    c.check(close(x * x + y * y, 1.0) && close(x, y), || format!("a square stick's corner: length 1 {x} {y}"));
    c.check(s.stick(f64::NAN, 0.5) == (0.0, 0.0) && s.stick(0.5, f64::INFINITY) == (0.0, 0.0), || "NaN and infinity: rest".into());
    let z = PadSettings { dead_zone: 0.0, ..s };
    c.check(z.stick(0.3, -0.4) == (0.3, -0.4), || "no dead zone: as it is".into());
    // looking: full deflection turns TURN_RATE degrees a second; the curve; sensitivity; inversion
    let rad = TURN_RATE * core::f64::consts::PI / 180.0;
    let (lx, ly) = s.look(1.0, 0.0);
    c.check(close(lx, rad) && ly == 0.0, || format!("full right: {lx} rad/s"));
    let (lx, _) = s.look(0.575, 0.0);
    c.check(close(lx, 0.25 * rad), || format!("half way after the dead zone, curve 2: a quarter ({lx})"));
    let (lx, _) = PadSettings { curve: 1.0, ..s }.look(0.575, 0.0);
    c.check(close(lx, 0.5 * rad), || format!("curve 1: half ({lx})"));
    let (lx, _) = PadSettings { curve: 3.0, ..s }.look(0.575, 0.0);
    c.check(close(lx, 0.125 * rad), || format!("curve 3: an eighth ({lx})"));
    let (lx, ly) = PadSettings { sensitivity: 2.0, invert_x: true, invert_y: true, ..s }.look(0.0, 1.0);
    c.check(lx == 0.0 && close(ly, -2.0 * rad), || format!("sensitivity 2, inverted: {lx} {ly}"));
    c.check(s.look(0.1, 0.0) == (0.0, 0.0), || "looking inside the dead zone: nothing".into());
}

/// the actions of the pad tests
struct P {
    mx: ActionId,
    my: ActionId,
    jump: ActionId,
    fire: ActionId,
    throttle: ActionId,
    look_x: ActionId,
    look_y: ActionId,
    up: ActionId,
}

fn pad_actions() -> (Input, P) {
    let mut m = ActionMap::new();
    let p = P {
        mx: m.add("move_x", ActionKind::Axis, &[Bind::key(KC_D), Bind::scaled(Source::Key(KC_A), -1.0), Bind::new(Source::PadAxis(PAD_LX as u8))]),
        my: m.add("move_y", ActionKind::Axis, &[Bind::key(KC_S), Bind::scaled(Source::Key(KC_W), -1.0), Bind::new(Source::PadAxis(PAD_LY as u8))]),
        jump: m.add("jump", ActionKind::Button, &[Bind::key(KC_SPACE), Bind::new(Source::PadButton(PAD_A as u8))]),
        fire: m.add("fire", ActionKind::Button, &[Bind::new(Source::Mouse(MOUSE_LEFT)), Bind::new(Source::PadButton(PAD_RT as u8))]),
        throttle: m.add("throttle", ActionKind::Axis, &[Bind::new(Source::PadButton(PAD_RT as u8))]),
        look_x: m.add("look_x", ActionKind::Motion, &[Bind::new(Source::MouseX), Bind::new(Source::PadAxis(PAD_RX as u8))]),
        look_y: m.add("look_y", ActionKind::Motion, &[Bind::new(Source::MouseY), Bind::new(Source::PadAxis(PAD_RY as u8))]),
        up: m.add("menu_up", ActionKind::Button, &[Bind::new(Source::PadDir(PAD_LY as u8, false))]),
    };
    (Input::new(m), p)
}

/// pad play: a pad button starts playing without the lock and fires nothing; taps; the lock and the mouse
fn pad_play(c: &mut Ctx) {
    let (mut i, p) = pad_actions();
    i.pad_frame(&[gp(0, 0, 0, 0, REST)], 0.016);
    c.check(i.active_pad().is_none() && !i.playing() && i.device() == Device::KeyboardMouse, || "a pad at rest: not active, not playing".into());
    // a stick while not playing: nothing starts, the axis says 0
    i.pad_frame(&[gp(0, 0, 0, 0, [0.9, 0.0, 0.0, 0.0])], 0.016);
    c.check(!i.playing() && i.axis(p.mx) == 0.0 && i.active_pad() == Some(0) && i.device() == Device::Pad, || "a stick: the pad is active, the game does not play".into());
    i.pad_frame(&[gp(0, 0, 0, 0, REST)], 0.016);
    // the press that starts playing
    i.pad_frame(&[gp(0, A, A, 0, REST)], 0.016);
    c.check(i.playing() && i.pad_playing() && !i.pressed(p.jump) && !i.down(p.jump) && !i.take_lock_request() && !i.pad_held(PAD_A), || "a pad press starts pad play, fires nothing".into());
    i.pad_frame(&[gp(0, A, 0, 0, REST)], 0.016);
    c.check(!i.down(p.jump), || "still held: nothing".into());
    i.pad_frame(&[gp(0, 0, 0, A, REST)], 0.016);
    c.check(!i.released(p.jump) && !i.down(p.jump), || "its release is swallowed too".into());
    let t = i.tick();
    c.check(!t.pressed(p.jump) && !t.released(p.jump), || "the tick saw nothing of it".into());
    i.end_frame();
    // playing: a press and a release
    i.pad_frame(&[gp(0, A, A, 0, REST)], 0.016);
    c.check(i.pressed(p.jump) && i.down(p.jump) && i.pad_held(PAD_A), || "a press fires".into());
    i.end_frame();
    i.pad_frame(&[gp(0, 0, 0, A, REST)], 0.016);
    c.check(i.released(p.jump) && !i.down(p.jump), || "its release".into());
    i.tick();
    i.end_frame();
    // a tap between two frames: pressed and released in one frame, one tick with down, pressed, released
    i.pad_frame(&[gp(0, 0, A, A, REST)], 0.016);
    c.check(i.pressed(p.jump) && i.released(p.jump) && !i.down(p.jump), || "a tap: pressed and released".into());
    let t = i.tick();
    c.check(t.down(p.jump) && t.pressed(p.jump) && t.released(p.jump), || format!("the tick of a tap: {:?}", t.get(p.jump)));
    i.end_frame();
    // held, let go and pressed again between two frames: released and pressed
    i.pad_frame(&[gp(0, A, A, 0, REST)], 0.016);
    i.end_frame();
    i.pad_frame(&[gp(0, A, A, A, REST)], 0.016);
    c.check(i.pressed(p.jump) && i.released(p.jump) && i.down(p.jump), || "up and down again: both, still held".into());
    i.end_frame();
    i.pad_frame(&[gp(0, 0, 0, A, REST)], 0.016);
    i.end_frame();
    // the mouse during pad play asks for the lock, its press is the lock's
    i.button(MOUSE_LEFT, true);
    c.check(i.take_lock_request() && !i.pressed(p.fire) && i.playing() && i.device() == Device::KeyboardMouse, || "a click in pad play asks for the lock, fires nothing".into());
    i.set_lock(PointerLock::Locked);
    i.button(MOUSE_LEFT, false);
    c.check(i.playing() && !i.pad_playing() && !i.take_pause() && !i.released(p.fire), || "the lock taken ends pad play without a pause".into());
    // a pad button held when the lock ends: let go, and it counts only after its release
    i.pad_frame(&[gp(0, A, A, 0, REST)], 0.016);
    c.check(i.down(p.jump) && i.device() == Device::Pad, || "locked: the pad presses".into());
    i.end_frame();
    i.set_lock(PointerLock::Off);
    c.check(!i.playing() && i.take_pause() && i.released(p.jump) && !i.down(p.jump), || "the lock's end: the pause lets the pad's button go".into());
    i.end_frame();
    i.set_lock(PointerLock::Locked);
    i.pad_frame(&[gp(0, A, 0, 0, REST)], 0.016);
    c.check(i.playing() && !i.down(p.jump) && !i.pressed(p.jump), || "back: the button held through the pause waits for its release".into());
    i.pad_frame(&[gp(0, 0, 0, A, REST)], 0.016);
    i.pad_frame(&[gp(0, A, A, 0, REST)], 0.016);
    c.check(i.pressed(p.jump) && i.down(p.jump), || "released and pressed again: it counts".into());
    i.end_frame();
    // keys still drive in pad play; Esc reaches the game (no lock)
    let (mut k, pk) = pad_actions();
    k.pad_frame(&[gp(1, B, B, 0, REST)], 0.016);
    k.key(KC_SPACE, true);
    c.check(k.pad_playing() && k.pressed(pk.jump) && k.device() == Device::KeyboardMouse, || "a key in pad play".into());
    // the pad gone during pad play: the pause
    k.end_frame();
    k.pad_frame(&[], 0.016);
    c.check(!k.playing() && k.take_pause() && k.pauses() == 1 && k.active_pad().is_none() && k.released(pk.jump), || "the pad gone in pad play: the pause".into());
    // capture: a pad's button and a stick's direction answer a settings page
    let (mut cap, pc) = pad_actions();
    cap.set_require_lock(false);
    cap.start_capture();
    cap.pad_frame(&[gp(0, 1 << PAD_X, 1 << PAD_X, 0, REST)], 0.016);
    c.check(cap.take_capture() == Some(Source::PadButton(PAD_X as u8)) && !cap.pad_held(PAD_X), || "captured a pad button".into());
    cap.start_capture();
    cap.pad_frame(&[gp(0, 1 << PAD_X, 0, 0, [0.9, 0.0, 0.0, 0.0])], 0.016);
    c.check(cap.take_capture() == Some(Source::PadDir(PAD_LX as u8, true)), || "captured a stick's direction".into());
    cap.start_capture();
    cap.pad_frame(&[gp(0, A, A, 0, [0.9, 0.0, 0.0, 0.0])], 0.016);
    c.check(cap.take_capture() == Some(Source::PadButton(PAD_A as u8)) && !cap.pressed(pc.jump) && !cap.down(pc.jump), || "a captured press drives nothing".into());
    // plat's event for a game with its own window
    c.check(cap.plat_event(&ev(plat::E_GAMEPAD, 0, 0, 0)), || "E_GAMEPAD is the input's (pads come with pad_frame)".into());
}

/// analog axes, triggers, stick directions, looking
fn pad_axes_and_look(c: &mut Ctx) {
    let (mut i, p) = pad_actions();
    i.set_require_lock(false);
    i.pad_frame(&[gp(0, 0, 0, 0, [0.575, -1.0, 0.0, 0.0])], 0.016);
    // (0.575, -1) has a length above 1: the direction kept, length 1
    let n = physics::math::sqrt(f(0.575) * f(0.575) + 1.0);
    c.check(close(i.axis(p.mx), f(0.575) / n) && close(i.axis(p.my), -1.0 / n) && i.down(p.mx), || format!("the left stick in two axes {} {}", i.axis(p.mx), i.axis(p.my)));
    i.pad_frame(&[gp(0, 0, 0, 0, [0.575, 0.0, 0.0, 0.0])], 0.016);
    c.check(close(i.axis(p.mx), beyond(0.575)) && (i.axis(p.mx) - 0.5).abs() < 1e-7 && i.axis(p.my) == 0.0, || format!("0.575 right: 0.5 ({})", i.axis(p.mx)));
    i.key(KC_D, true);
    c.check(i.axis(p.mx) == 1.0, || "D and the stick: kept at 1".into());
    i.key(KC_D, false);
    i.key(KC_A, true);
    c.check(close(i.axis(p.mx), beyond(0.575) - 1.0), || format!("A (-1) and the stick (0.5): -0.5 ({})", i.axis(p.mx)));
    i.key(KC_A, false);
    i.pad_frame(&[gp(0, 0, 0, 0, [0.1, 0.1, 0.0, 0.0])], 0.016);
    c.check(i.axis(p.mx) == 0.0 && !i.down(p.mx), || "inside the dead zone: 0, not held".into());
    // a trigger: analog in an Axis, a press above the threshold
    let mut g = gp(0, 0, 0, 0, REST);
    g.buttons[PAD_RT] = 0.1;
    i.pad_frame(&[g], 0.016);
    c.check(close(i.axis(p.throttle), 0.1f32 as f64) && !i.down(p.fire), || format!("RT 0.1: throttle 0.1, no fire ({})", i.axis(p.throttle)));
    let mut g = gp(0, RT, RT, 0, REST);
    g.buttons[PAD_RT] = 0.8;
    i.pad_frame(&[g], 0.016);
    c.check(close(i.axis(p.throttle), 0.8f32 as f64) && i.pressed(p.fire) && i.down(p.fire), || "RT 0.8: throttle and fire".into());
    i.end_frame();
    i.pad_frame(&[gp(0, 0, 0, RT, REST)], 0.016);
    c.check(i.released(p.fire) && i.axis(p.throttle) == 0.0, || "RT let go".into());
    i.end_frame();
    // a stick's direction as a button, with its hysteresis
    let up = |v: f32| gp(0, 0, 0, 0, [0.0, v, 0.0, 0.0]);
    i.pad_frame(&[up(-0.5)], 0.016); // (0.5 - 0.15) / 0.85 = 0.41: not yet
    c.check(!i.down(p.up), || "up 0.41: not yet pressed".into());
    i.pad_frame(&[up(-0.7)], 0.016); // 0.65
    c.check(i.pressed(p.up) && i.down(p.up), || "up 0.65: pressed".into());
    i.pad_frame(&[up(-0.5)], 0.016); // 0.41: held still
    c.check(i.down(p.up), || format!("up 0.41: still held (let go below {PAD_DIR_RELEASE}, pressed from {PAD_DIR_PRESS})"));
    i.pad_frame(&[up(-0.45)], 0.016); // 0.35
    c.check(!i.down(p.up) && i.released(p.up), || "up 0.35: let go".into());
    i.end_frame();
    i.tick();
    // looking: the right stick at full deflection for 0.25 s turns 45 degrees at TURN_RATE 180
    let rad = TURN_RATE * core::f64::consts::PI / 180.0;
    i.pad_frame(&[gp(0, 0, 0, 0, [0.0, 0.0, 1.0, 0.0])], 0.25);
    c.check(close(i.motion_of(p.look_x), 0.25 * rad) && i.motion_of(p.look_y) == 0.0, || format!("full right 0.25 s: {} rad", i.motion_of(p.look_x)));
    i.end_frame();
    let half = beyond(0.575) * beyond(0.575) * rad; // (half way after the dead zone, curve 2: a quarter of the rate)
    i.pad_frame(&[gp(0, 0, 0, 0, [0.0, 0.0, 0.575, 0.0])], 0.125);
    c.check(close(i.motion_of(p.look_x), half * 0.125) && (half / rad - 0.25).abs() < 1e-7, || format!("half way (curve 2) 0.125 s: {}", i.motion_of(p.look_x)));
    let t = i.tick();
    c.check(close(t.value(p.look_x), 0.25 * rad + half * 0.125), || format!("the tick: both frames {}", t.value(p.look_x)));
    i.end_frame();
    // the mouse and the stick in one Motion add up; the frame's time is kept at 0.25 s
    i.motion(100.0, 0.0);
    i.pad_frame(&[gp(0, 0, 0, 0, [0.0, 0.0, 0.0, 1.0])], 10.0);
    c.check(close(i.motion_of(p.look_y), 0.25 * rad) && close(i.motion_of(p.look_x), 100.0 * k(1.0)), || "a long frame turns at most 0.25 s; the mouse beside".into());
    i.end_frame();
    i.set_pad_settings(PadSettings { invert_y: true, sensitivity: 0.5, ..PadSettings::DEFAULT });
    i.pad_frame(&[gp(0, 0, 0, 0, [0.0, 0.0, 0.0, 1.0])], 0.1);
    c.check(close(i.motion_of(p.look_y), -0.5 * rad * 0.1), || format!("inverted, sensitivity 0.5: {}", i.motion_of(p.look_y)));
    i.end_frame();
    i.pad_frame(&[gp(0, 0, 0, 0, [0.0, 0.0, 0.0, 1.0])], f64::NAN);
    c.check(i.motion_of(p.look_y) == 0.0, || "a frame's time NaN: no turn".into());
    // not playing: a stick drives nothing
    let (mut np, pn) = pad_actions();
    np.pad_frame(&[gp(0, 0, 0, 0, [1.0, 0.0, 1.0, 0.0])], 0.1);
    c.check(np.axis(pn.mx) == 0.0 && np.motion_of(pn.look_x) == 0.0 && !np.playing(), || "not playing: no axis, no turn".into());
}

/// which pad drives the input: the one used last, or the pinned one
fn pad_which(c: &mut Ctx) {
    let (mut i, p) = pad_actions();
    i.set_require_lock(false);
    i.pad_frame(&[gp(0, A, A, 0, REST), gp(1, 0, 0, 0, REST)], 0.016);
    c.check(i.active_pad() == Some(0) && i.down(p.jump), || "pad 0 pressed: it drives".into());
    i.end_frame();
    // pad 1's stick drifting out of its dead zone does not take over
    i.pad_frame(&[gp(0, A, 0, 0, REST), gp(1, 0, 0, 0, [0.0, 0.3, 0.0, 0.0])], 0.016);
    c.check(i.active_pad() == Some(0) && i.axis(p.my) == 0.0, || "pad 1's stick does not take over".into());
    // pad 1's button does: pad 0's A goes, pad 1's own press counts
    i.pad_frame(&[gp(0, A, 0, 0, REST), gp(1, B, B, 0, [0.0, 0.3, 0.0, 0.0])], 0.016);
    c.check(i.active_pad() == Some(1) && i.released(p.jump) && !i.down(p.jump) && close(i.axis(p.my), beyond(0.3)), || format!("pad 1 pressed B: it drives now ({:?}, {})", i.active_pad(), i.axis(p.my)));
    i.end_frame();
    // a button held on pad 0 when it takes over again waits for its release; its new press counts
    i.pad_frame(&[gp(0, A | RT, RT, 0, REST), gp(1, 0, 0, B, REST)], 0.016);
    c.check(i.active_pad() == Some(0) && i.down(p.fire) && !i.down(p.jump), || "back to pad 0: RT pressed counts, A held before waits".into());
    i.end_frame();
    // pinned: only pad 1, whatever pad 0 does
    i.pin_pad(Some(1));
    i.pad_frame(&[gp(0, 0, 0, A | RT, REST), gp(1, 0, 0, 0, REST)], 0.016);
    i.pad_frame(&[gp(0, A, A, 0, REST), gp(1, 0, 0, 0, REST)], 0.016);
    c.check(i.active_pad() == Some(1) && i.pinned_pad() == Some(1) && !i.down(p.jump), || "pinned to pad 1: pad 0 drives nothing".into());
    i.pad_frame(&[gp(0, A, 0, 0, REST)], 0.016);
    c.check(i.active_pad().is_none(), || "the pinned pad gone: none drives".into());
    i.pin_pad(None);
    i.pad_frame(&[gp(0, 0, 0, A, REST)], 0.016);
    i.pad_frame(&[gp(0, A, A, 0, REST)], 0.016);
    c.check(i.active_pad() == Some(0) && i.down(p.jump), || "unpinned: the one used".into());
}

fn pad_settings(c: &mut Ctx) {
    let mut ini = Ini::new();
    PadSettings::DEFAULT.store(&mut ini, "input");
    c.check(ini.section("input").is_empty() && PadSettings::load(&ini, "input") == PadSettings::DEFAULT, || "default pad settings store nothing".into());
    let s = PadSettings { sensitivity: 1.75, dead_zone: 0.2, curve: 1.5, invert_x: true, invert_y: false };
    s.store(&mut ini, "input");
    c.check(ini.get("input", "pad_sensitivity") == Some("1.75") && ini.get("input", "pad_dead_zone") == Some("0.2") && ini.get("input", "pad_curve") == Some("1.5")
            && ini.get("input", "pad_invert_x") == Some("true") && ini.get("input", "pad_invert_y").is_none(), || format!("stored: {:?}", ini.section("input")));
    let back = Ini::parse(&ini.to_bytes());
    c.check(PadSettings::load(&back, "input") == s, || "through the file's text and back".into());
    c.check(MouseSettings::load(&back, "input") == MouseSettings::DEFAULT, || "the pad's lines leave the mouse at its defaults".into());
    ini.set("input", "pad_dead_zone", "5");
    ini.set("input", "pad_curve", "0");
    ini.set("input", "pad_sensitivity", "x");
    let l = PadSettings::load(&ini, "input");
    c.check(l.dead_zone == 0.9 && l.curve == 1.0 && l.sensitivity == 1.0 && l.invert_x, || format!("clamped, unreadable: {l:?}"));
    // the input stores and loads them beside its bindings and the mouse
    let (mut i, p) = pad_actions();
    i.set_pad_settings(PadSettings { dead_zone: 0.25, ..PadSettings::DEFAULT });
    i.set_mouse(MouseSettings { sensitivity: 3.0, ..MouseSettings::DEFAULT });
    i.edit_map(|m| m.set_binds(p.jump, &[Bind::new(Source::PadButton(PAD_B as u8))]));
    let mut ini = Ini::new();
    i.store(&mut ini, "input");
    c.check(ini.get("input", "bind.jump") == Some("PadB") && ini.get("input", "pad_dead_zone") == Some("0.25") && ini.get("input", "sensitivity") == Some("3"),
            || format!("the input's section: {:?}", ini.section("input")));
    let (mut j, _) = pad_actions();
    let bad = j.load(&Ini::parse(&ini.to_bytes()), "input");
    c.check(bad == 0 && j.pad_settings().dead_zone == 0.25 && j.mouse().sensitivity == 3.0 && same_binds(i.map(), j.map()), || "loaded beside the bindings and the mouse".into());
    // a pad source a kind does not take is left out and counted
    ini.set("input", "bind.jump", "PadLeftX PadA");
    let (mut j, pj) = pad_actions();
    let bad = j.load(&ini, "input");
    c.check(bad == 1 && j.map().binds(pj.jump) == [Bind::new(Source::PadButton(PAD_A as u8))], || format!("a Button refuses a stick's axis: {bad}"));
}
