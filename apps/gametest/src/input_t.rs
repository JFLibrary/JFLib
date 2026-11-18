// input_t -- the group "input": names and bindings, the map, the settings file (base::Ini and
// base::Settings), the pointer lock's click and end, frames against ticks, the wheel, capture, plat's
// events, the factor per platform; every case against the rule of docs/GAMECORE.md written out

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::{Ini, Schema, Settings};
use game::input::{self, MOUSE_LEFT, MOUSE_RIGHT, MOUSE_X2, platform_motion_factor};
use game::{ActionId, ActionKind, ActionMap, Bind, Input, MouseSettings, Source};
use plat::{KC_A, KC_D, KC_DOWN, KC_Q, KC_S, KC_SPACE, KC_UP, KC_W, PointerLock};

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
