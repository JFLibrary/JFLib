// The phone's sensors and location (plat::sensors): "Wasserwaage" (a bubble level: flat a round vial,
// on its edge a tube), "Handylage" (pitch, roll, yaw, the phone drawn in the room), "Kompass" (the
// heading on a turning dial), "GPS" (latitude, longitude, accuracy, altitude, speed, bearing, the
// fix's age; no map, no network).  A page starts what it needs when it opens (Sensors::enter) and
// stops it when it closes; the library stops everything in the background (UIC_SUSPEND).  The
// location starts from the switch: Android's permission and the browser's prompt ask then.
// With jfgui_SELFTEST=sensors (the start intent's extras on Android, debug builds;
// apps/androidGallery/tests/android_sensors.py) the state goes to data://sensors-state.txt as
// "key=value" lines.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::Cell;

use plat::sensors::{self as ps, Attitude, FixSource, LocationState, LocationWatch, SensorState, SensorWatch, SENSOR_ACCEL, SENSOR_GRAVITY, SENSOR_GYRO, SENSOR_MAGNET, SENSOR_ROTATION};
use ui::*;

use crate::nav::Page;

const ID_LEVEL: u32 = 10000;
const ID_ATTITUDE: u32 = 10001;
const ID_COMPASS: u32 = 10002;
const ID_LOC: u32 = 10010;
const TIMER_AGE: u32 = 10020;
const STATE: &str = "data://sensors-state.txt";
/// degrees at the end of the level's scale
const LEVEL_RANGE: f32 = 30.0;
/// within this many degrees the level says "waagerecht"
const LEVEL_OK: f32 = 0.5;

const fn mask_of(page: Page) -> u32 {
    match page {
        Page::Level => SENSOR_ACCEL | SENSOR_GRAVITY | SENSOR_ROTATION,
        Page::Attitude => SENSOR_ACCEL | SENSOR_GRAVITY | SENSOR_GYRO | SENSOR_MAGNET | SENSOR_ROTATION,
        Page::Compass => SENSOR_ACCEL | SENSOR_GRAVITY | SENSOR_MAGNET | SENSOR_ROTATION,
        _ => 0,
    }
}

const SENSORS: [(u32, &str, &str, &str); 5] = [
    (SENSOR_ACCEL, "Beschleunigung", "m/s²", "accel"),
    (SENSOR_GRAVITY, "Schwerkraft", "m/s²", "gravity"),
    (SENSOR_GYRO, "Drehrate", "rad/s", "gyro"),
    (SENSOR_MAGNET, "Magnetfeld", "µT", "magnet"),
    (SENSOR_ROTATION, "Drehlage", "", "rot"),
];

fn state_text(s: SensorState) -> &'static str {
    match s {
        SensorState::Off => "aus",
        SensorState::Waiting => "wartet auf Daten",
        SensorState::Running => "läuft",
        SensorState::Paused => "pausiert",
        SensorState::Denied => "nicht erlaubt",
        SensorState::Unavailable => "nicht verfügbar",
    }
}

fn state_key(s: SensorState) -> &'static str {
    match s {
        SensorState::Off => "off",
        SensorState::Waiting => "waiting",
        SensorState::Running => "running",
        SensorState::Paused => "paused",
        SensorState::Denied => "denied",
        SensorState::Unavailable => "unavailable",
    }
}

// a number for the user; no "-0,0"
fn num(ui: &Ui, v: f64, decimals: u32) -> String {
    let t = ui.fmt_f64(v, decimals);
    if t.starts_with('-') && t.bytes().all(|b| matches!(b, b'-' | b'0' | b',' | b'.')) {
        return String::from(&t[1..]);
    }
    t
}

fn fx(v: f32) -> i32 {
    (v * 256.0) as i32
}

fn disc(g: &mut Gfx, cx: f32, cy: f32, r: f32, color: u32) {
    g.dc.set_color(color);
    g.shapes().fill_circle_fx(fx(cx), fx(cy), fx(r));
}

fn ring(g: &mut Gfx, cx: f32, cy: f32, r: f32, t: f32, color: u32) {
    g.dc.set_color(color);
    g.shapes().stroke_circle_fx(fx(cx), fx(cy), fx(r), fx(t));
}

fn line(g: &mut Gfx, a: (f32, f32), b: (f32, f32), t: f32, color: u32) {
    g.dc.set_color(color);
    g.shapes().line_fx(fx(a.0), fx(a.1), fx(b.0), fx(b.1), fx(t));
}

fn poly(g: &mut Gfx, pts: &[(f32, f32)], color: u32) {
    let path = &mut g.text.path;
    path.reset();
    path.set_origin(0, 0);
    for (k, p) in pts.iter().enumerate() {
        if k == 0 {
            path.move_f(p.0, p.1);
        } else {
            path.line_f(p.0, p.1);
        }
    }
    path.close();
    g.dc.set_color(color);
    g.text.path.fill(&mut g.text.ras, g.dc, g.sf, 0);
}

// text centred on a point
fn label(g: &mut Gfx, s: &str, cx: f32, cy: f32, px: f32, color: u32) {
    let was = g.text.get_font();
    g.text.set_font(was.0, fx(px));
    let w = g.text.width(s);
    let y = cy as i32 + (g.text.ascent() - g.text.descent()) / 2;
    g.dc.set_color(color);
    g.text.draw(g.dc, g.sf, cx as i32 - w / 2, y, s.as_bytes());
    g.text.set_font(was.0, was.1);
}

fn sincos(deg: f32) -> (f32, f32) {
    let (c, s) = deg_sincos(deg as f64);
    (s as f32, c as f32)
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Look {
    dark: bool,
    known: bool,
}

impl Look {
    fn face(self) -> u32 {
        if self.dark { 0xFF2B2B2B } else { 0xFFFFFFFF }
    }
    fn line(self) -> u32 {
        if self.dark { 0xFF9A9A9A } else { 0xFF6E6E6E }
    }
    fn text(self) -> u32 {
        if self.dark { 0xFFF0F0F0 } else { 0xFF1A1A1A }
    }
}

// ---------------------------------------------------------------- level

/// x, y: -1 .. 1 along the screen; edge: the phone stands on an edge: a tube along the axis that
/// lies in the room (tube_y: along the screen's height)
#[derive(Clone, Copy, Default, PartialEq)]
struct LevelState {
    x: f32,
    y: f32,
    edge: bool,
    tube_y: bool,
    ok: bool,
    look: Look,
}

#[derive(Default)]
struct LevelDraw(Cell<LevelState>);

impl Draw for LevelDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let s = self.0.get();
        let (x0, y0, w, h) = (r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32);
        let (cx, cy) = (x0 + w / 2.0, y0 + h / 2.0);
        let rad = w.min(h) / 2.0 - 4.0;
        let vial = if s.look.dark { 0xFF30401F } else { 0xFFE3EFC4 };
        let bubble = if !s.look.known {
            0
        } else if s.ok {
            0xFF107C10
        } else if s.look.dark {
            0xFFF2F7E4
        } else {
            0xFF8FB339
        };
        let t = (rad / 100.0).max(1.0);
        if s.edge {
            // a tube
            let (hw, hh) = if s.tube_y { (rad * 0.2, rad) } else { (rad, rad * 0.2) };
            let (tx, ty, tw, th) = (cx - hw, cy - hh, hw * 2.0, hh * 2.0);
            let corner = hw.min(hh) as i32;
            g.dc.set_color(vial);
            g.shapes().fill_rrect(tx as i32, ty as i32, tw as i32, th as i32, corner);
            g.dc.set_color(s.look.line());
            g.shapes().stroke_rrect(tx as i32, ty as i32, tw as i32, th as i32, corner, (t * 2.0) as i32);
            let gap = rad * 0.22;
            for side in [-1.0f32, 1.0] {
                if s.tube_y {
                    line(g, (cx - hw, cy + side * gap), (cx + hw, cy + side * gap), t * 2.0, s.look.line());
                } else {
                    line(g, (cx + side * gap, cy - hh), (cx + side * gap, cy + hh), t * 2.0, s.look.line());
                }
            }
            if bubble != 0 {
                let room = rad - hw.min(hh);
                let (bx, by) = if s.tube_y { (cx, cy - s.y * room) } else { (cx + s.x * room, cy) };
                disc(g, bx, by, hw.min(hh) * 0.75, bubble);
                ring(g, bx, by, hw.min(hh) * 0.75, t * 1.5, s.look.line());
            }
        } else {
            // a round vial with the circle the bubble has to meet
            disc(g, cx, cy, rad, vial);
            ring(g, cx, cy, rad, t * 2.0, s.look.line());
            ring(g, cx, cy, rad * 0.5, t, s.look.line());
            let br = rad * 0.16;
            ring(g, cx, cy, br * 1.2, t * 2.0, s.look.line());
            line(g, (cx - rad, cy), (cx + rad, cy), t, s.look.line());
            line(g, (cx, cy - rad), (cx, cy + rad), t, s.look.line());
            if bubble != 0 {
                let room = rad - br;
                // not beyond the wall
                let n = s.x * s.x + s.y * s.y;
                let f = if n > 1.0 { 1.0 / rt_sqrt(n) } else { 1.0 };
                let (bx, by) = (cx + s.x * f * room, cy - s.y * f * room);
                disc(g, bx, by, br, bubble);
                ring(g, bx, by, br, t * 1.5, s.look.line());
            }
        }
    }
}

// Newton's root: enough for a screen position
fn rt_sqrt(n: f32) -> f32 {
    let mut x = if n > 1.0 { n } else { 1.0 };
    for _ in 0..6 {
        x = 0.5 * (x + n / x);
    }
    x
}

// ---------------------------------------------------------------- the phone in the room

#[derive(Clone, Copy, Default, PartialEq)]
struct PhoneState {
    pitch: f32,
    roll: f32,
    yaw: f32,
    look: Look,
}

#[derive(Default)]
struct PhoneDraw(Cell<PhoneState>);

impl Draw for PhoneDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let s = self.0.get();
        let (x0, y0, w, h) = (r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32);
        let (cx, cy) = (x0 + w / 2.0, y0 + h / 2.0);
        let unit = w.min(h) * 0.34;
        // the camera looks down from the south, 55 degrees above the floor
        let (se, ce) = sincos(55.0);
        // the floor, north at its top
        let floor = if s.look.dark { 0xFF262626 } else { 0xFFEDEDED };
        g.dc.set_color(floor);
        g.shapes().fill_ellipse_fx(fx(cx), fx(cy), fx(unit * 1.4), fx(unit * 1.4 * se));
        label(g, "N", cx, cy - unit * 1.4 * se - unit * 0.1, unit * 0.16, s.look.text());
        // device -> room: yaw * pitch * roll
        let (sp, cp) = sincos(s.pitch);
        let (sr, cr) = sincos(s.roll);
        let (sy, cy2) = sincos(-s.yaw);
        let m: [[f32; 3]; 3] = [
            [cr, 0.0, sr],
            [sp * sr, cp, -sp * cr],
            [-cp * sr, sp, cp * cr],
        ];
        let turn = |v: [f32; 3]| -> [f32; 3] {
            let d = [
                m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
                m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
                m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
            ];
            [cy2 * d[0] - sy * d[1], sy * d[0] + cy2 * d[1], d[2]]
        };
        let project = |v: [f32; 3]| -> (f32, f32) { (cx + v[0] * unit, cy - (v[1] * se + v[2] * ce) * unit) };
        let seen = |n: [f32; 3]| -> bool { -n[1] * ce + n[2] * se > 0.0 };
        let (hw, hh, hd) = (0.5f32, 1.0f32, 0.07f32);
        // (normal, four corners, what the face is)
        let faces: [([f32; 3], [[f32; 3]; 4], u8); 6] = [
            ([0.0, 0.0, 1.0], [[-hw, hh, hd], [hw, hh, hd], [hw, -hh, hd], [-hw, -hh, hd]], 0),
            ([0.0, 0.0, -1.0], [[-hw, -hh, -hd], [hw, -hh, -hd], [hw, hh, -hd], [-hw, hh, -hd]], 1),
            ([1.0, 0.0, 0.0], [[hw, hh, hd], [hw, hh, -hd], [hw, -hh, -hd], [hw, -hh, hd]], 2),
            ([-1.0, 0.0, 0.0], [[-hw, -hh, hd], [-hw, -hh, -hd], [-hw, hh, -hd], [-hw, hh, hd]], 2),
            ([0.0, 1.0, 0.0], [[-hw, hh, -hd], [hw, hh, -hd], [hw, hh, hd], [-hw, hh, hd]], 3),
            ([0.0, -1.0, 0.0], [[-hw, -hh, hd], [hw, -hh, hd], [hw, -hh, -hd], [-hw, -hh, -hd]], 2),
        ];
        for (n, c, kind) in faces {
            if !seen(turn(n)) {
                continue;
            }
            let p = [project(turn(c[0])), project(turn(c[1])), project(turn(c[2])), project(turn(c[3]))];
            let fill = match kind {
                0 => 0xFF1F3A5F,
                1 => 0xFF4A4A4A,
                3 => 0xFFD83B01, // the top edge
                _ => 0xFF707070,
            };
            poly(g, &p, fill);
            if kind == 0 {
                // the screen, a mark at the top
                let inset = |x: f32, y: f32| project(turn([x, y, hd]));
                let screen = [inset(-hw * 0.86, hh * 0.86), inset(hw * 0.86, hh * 0.86), inset(hw * 0.86, -hh * 0.9), inset(-hw * 0.86, -hh * 0.9)];
                poly(g, &screen, if s.look.known { 0xFF0078D4 } else { 0xFF5A7A99 });
                let top = [inset(-hw * 0.3, hh * 0.72), inset(hw * 0.3, hh * 0.72), inset(0.0, hh * 0.46)];
                poly(g, &top, 0xFFFFFFFF);
            }
            if kind == 1 {
                let dot = project(turn([-hw * 0.5, hh * 0.72, -hd]));
                disc(g, dot.0, dot.1, unit * 0.09, 0xFF202020);
            }
        }
    }
}

// ---------------------------------------------------------------- compass

#[derive(Clone, Copy, Default, PartialEq)]
struct CompassState {
    /// NaN: no direction
    heading: f32,
    look: Look,
}

#[derive(Default)]
struct CompassDraw(Cell<CompassState>);

impl Draw for CompassDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let s = self.0.get();
        let (x0, y0, w, h) = (r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32);
        let (cx, cy) = (x0 + w / 2.0, y0 + h / 2.0);
        let rad = w.min(h) / 2.0 - 4.0;
        let t = (rad / 100.0).max(1.0);
        disc(g, cx, cy, rad, s.look.face());
        ring(g, cx, cy, rad, t * 2.0, s.look.line());
        let known = !s.heading.is_nan();
        let turn = if known { s.heading } else { 0.0 };
        let at = |deg: f32, dist: f32| -> (f32, f32) {
            let (a, b) = sincos(deg - turn);
            (cx + a * dist, cy - b * dist)
        };
        // the scale turns against the heading
        for d in 0..36 {
            let deg = d as f32 * 10.0;
            let long = d % 9 == 0;
            let inner = rad * if long { 0.84 } else if d % 3 == 0 { 0.88 } else { 0.91 };
            line(g, at(deg, inner), at(deg, rad * 0.96), if long { t * 3.0 } else { t }, s.look.line());
        }
        for (deg, name) in [(0.0, "N"), (90.0, "O"), (180.0, "S"), (270.0, "W")] {
            let p = at(deg, rad * 0.7);
            label(g, name, p.0, p.1, rad * 0.16, if deg == 0.0 { 0xFFD13438 } else { s.look.text() });
        }
        if known {
            // north's needle
            let tip = at(0.0, rad * 0.52);
            let tail = at(180.0, rad * 0.52);
            let (a, b) = sincos(90.0 - turn);
            let side = (a * rad * 0.07, -b * rad * 0.07);
            poly(g, &[tip, (cx + side.0, cy + side.1), (cx - side.0, cy - side.1)], 0xFFD13438);
            poly(g, &[tail, (cx - side.0, cy - side.1), (cx + side.0, cy + side.1)], s.look.line());
            disc(g, cx, cy, rad * 0.04, s.look.text());
        }
        // what the phone points at: fixed at the top
        poly(g, &[(cx, cy - rad * 0.98), (cx - rad * 0.06, cy - rad * 0.86), (cx + rad * 0.06, cy - rad * 0.86)], 0xFF0078D4);
    }
}

// ---------------------------------------------------------------- the pages

const COMPASS_POINTS: [&str; 8] = ["N", "NO", "O", "SO", "S", "SW", "W", "NW"];

pub struct Sensors {
    page: Page,
    watch: Option<SensorWatch>,
    location: Option<LocationWatch>,
    loc_on: i32,
    level: Rc<LevelDraw>,
    phone: Rc<PhoneDraw>,
    compass: Rc<CompassDraw>,
    version: u32,
    selftest: Option<bool>,
    uic_sensor: u32,
    uic_location: u32,
    suspend: u32,
    written: String,
    written_key: String,
    written_at: u32,
}

impl Default for Sensors {
    fn default() -> Self {
        Self {
            page: Page::Home,
            watch: None,
            location: None,
            loc_on: 0,
            level: Rc::default(),
            phone: Rc::default(),
            compass: Rc::default(),
            version: 1,
            selftest: None,
            uic_sensor: 0,
            uic_location: 0,
            suspend: 0,
            written: String::new(),
            written_key: String::new(),
            written_at: 0,
        }
    }
}

impl Sensors {
    /// a page opens: its sensors start, the others stop (what both need runs on)
    pub fn enter(&mut self, page: Page) {
        self.page = page;
        let mask = mask_of(page);
        self.watch = (mask != 0).then(|| ps::watch(mask));
        self.location = if page == Page::Gps && self.loc_on != 0 { Some(ps::location_watch()) } else { None };
    }

    /// the library's commands: count them, a frame -> handled
    pub fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            UIC_SENSOR => self.uic_sensor += 1,
            UIC_LOCATION => self.uic_location += 1,
            UIC_TIMER if arg == TIMER_AGE => {}
            UIC_SUSPEND => {
                self.suspend = arg;
                self.save(ui);
                return false; // the card "06 System" counts it too
            }
            UIC_PERMISSION => {
                // the answer to an `ask` line of a manifest: try again
                if arg == 1 && self.page != Page::Home {
                    self.enter(self.page);
                }
                return false;
            }
            _ => return false,
        }
        ui.invalidate();
        true
    }

    fn look(ui: &Ui, known: bool) -> Look {
        Look { dark: ui.theme_dark(), known }
    }

    // a square canvas as wide as the page
    fn canvas(&mut self, ui: &mut Ui, id: u32, d: DrawRef, changed: bool) {
        if changed {
            self.version = self.version.wrapping_add(1);
        }
        let side = (ui.client_size().0 - 2 * 16).clamp(160, 360);
        ui.box_begin(0, BOX_H | BOX_CENTER, 0);
        ui.next_size(side, side);
        ui.next_flags(ST_NOFOCUS | (CUR_ARROW << HB_CURSOR_SHIFT));
        ui.canvas(id, Some(d), 0, self.version);
        ui.box_end();
    }

    fn rows(&self, ui: &mut Ui, mask: u32) {
        ui.box_begin(0, BOX_V | BOX_CARD | PAD_12, 6);
        for (bit, name, unit, _) in SENSORS {
            if mask & bit == 0 {
                continue;
            }
            let st = ps::state(bit);
            let values = match ps::read(bit).filter(|_| st != SensorState::Unavailable) {
                Some(r) if bit == SENSOR_ROTATION => format!(
                    "{} {} {} {}",
                    num(ui, r.v[0] as f64, 3),
                    num(ui, r.v[1] as f64, 3),
                    num(ui, r.v[2] as f64, 3),
                    num(ui, r.v[3] as f64, 3)
                ),
                Some(r) => format!("{} {} {} {unit}", num(ui, r.v[0] as f64, 2), num(ui, r.v[1] as f64, 2), num(ui, r.v[2] as f64, 2)),
                None => String::new(),
            };
            ui.text(TS_BODY | TX_WRAP, &format!("{name}: {}", state_text(st)));
            if !values.is_empty() {
                ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, &values);
            }
        }
        ui.box_end();
    }

    // a line when nothing can be shown
    fn missing(ui: &mut Ui, mask: u32) {
        let mut all_gone = true;
        let mut paused = false;
        let mut denied = false;
        for (bit, ..) in SENSORS {
            if mask & bit != 0 {
                let st = ps::state(bit);
                all_gone &= matches!(st, SensorState::Unavailable | SensorState::Denied);
                paused |= st == SensorState::Paused;
                denied |= st == SensorState::Denied;
            }
        }
        if denied {
            ui.infobar(ID_LEVEL + 100, IB_ERROR, "Nicht erlaubt", "Die App darf die Bewegungssensoren nicht lesen.");
        } else if all_gone {
            ui.infobar(ID_LEVEL + 100, IB_WARNING, "Nicht verfügbar", "Dieses Gerät hat keine passenden Sensoren.");
        } else if paused {
            ui.text(TS_BODY | TX_SECONDARY, "Pausiert, solange die App im Hintergrund ist.");
        }
    }

    pub fn level(&mut self, ui: &mut Ui) {
        let a = ps::attitude();
        let was = self.level.0.get();
        let mut s = LevelState { look: Self::look(ui, a.is_some()), ..was };
        if let Some(a) = a {
            let (dx, dy) = (a.side, a.pitch);
            let edge = a.up[2].abs() < 0.5;
            let tube_y = edge && a.up[0].abs() > a.up[1].abs();
            let ok = if !edge { dx.abs() < LEVEL_OK && dy.abs() < LEVEL_OK } else if tube_y { dy.abs() < LEVEL_OK } else { dx.abs() < LEVEL_OK };
            s = LevelState { x: (dx / LEVEL_RANGE).clamp(-1.0, 1.0), y: (dy / LEVEL_RANGE).clamp(-1.0, 1.0), edge, tube_y, ok, look: s.look };
        }
        self.level.0.set(s);
        let d: DrawRef = self.level.clone();
        self.canvas(ui, ID_LEVEL, d, s != was);
        Self::missing(ui, mask_of(Page::Level));
        match a {
            Some(a) => {
                let verdict = if s.ok { "Waagerecht" } else if s.edge { "Auf der Kante" } else { "Geneigt" };
                ui.text(TS_SUBTITLE, verdict);
                let (dx, dy) = (num(ui, a.side as f64, 1), num(ui, a.pitch as f64, 1));
                ui.text(TS_BODY | TX_WRAP, &format!("Quer: {dx}°   Längs: {dy}°"));
            }
            None => ui.text(TS_SUBTITLE, "Keine Daten"),
        }
        self.rows(ui, mask_of(Page::Level));
        self.save(ui);
    }

    pub fn attitude(&mut self, ui: &mut Ui) {
        let a = ps::attitude();
        let was = self.phone.0.get();
        let s = match a {
            Some(a) => PhoneState { pitch: a.pitch, roll: a.roll, yaw: a.heading.unwrap_or(0.0), look: Self::look(ui, true) },
            None => PhoneState { look: Self::look(ui, false), ..was },
        };
        self.phone.0.set(s);
        let d: DrawRef = self.phone.clone();
        self.canvas(ui, ID_ATTITUDE, d, s != was);
        Self::missing(ui, mask_of(Page::Attitude));
        match a {
            Some(a) => {
                ui.text(TS_BODY | TX_WRAP, &format!("Nicken: {}°", num(ui, a.pitch as f64, 1)));
                ui.text(TS_BODY | TX_WRAP, &format!("Rollen: {}°", num(ui, a.roll as f64, 1)));
                let yaw = a.heading.map_or(String::from("keine Angabe"), |y| format!("{}°{}", num(ui, y as f64, 1), if a.absolute { "" } else { " (relativ)" }));
                ui.text(TS_BODY | TX_WRAP, &format!("Gieren: {yaw}"));
            }
            None => ui.text(TS_SUBTITLE, "Keine Daten"),
        }
        self.rows(ui, mask_of(Page::Attitude));
        self.save(ui);
    }

    pub fn compass(&mut self, ui: &mut Ui) {
        let a = ps::attitude();
        let heading = a.and_then(|a| a.heading);
        let was = self.compass.0.get();
        let s = CompassState { heading: heading.unwrap_or(f32::NAN), look: Self::look(ui, heading.is_some()) };
        self.compass.0.set(s);
        let same = (s.heading == was.heading || (s.heading.is_nan() && was.heading.is_nan())) && s.look == was.look;
        let d: DrawRef = self.compass.clone();
        self.canvas(ui, ID_COMPASS, d, !same);
        Self::missing(ui, mask_of(Page::Compass));
        match heading {
            Some(h) => {
                let point = COMPASS_POINTS[((h + 22.5) / 45.0) as usize % 8];
                ui.text(TS_SUBTITLE, &format!("{}° {point}", num(ui, h as f64, 0)));
                if !a.is_some_and(|a| a.absolute) {
                    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Relativ zum Start: ohne Magnetfeld kein Norden.");
                }
            }
            None => ui.text(TS_SUBTITLE, "Keine Richtung"),
        }
        // Android: 0 = "unreliable", the magnetometer asks for the figure eight
        if ps::read(SENSOR_MAGNET).is_some_and(|r| r.accuracy == 0) && ps::read(SENSOR_ROTATION).is_none() {
            ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Ungenau: das Telefon in einer Acht bewegen.");
        }
        self.rows(ui, mask_of(Page::Compass));
        self.save(ui);
    }

    pub fn gps(&mut self, ui: &mut Ui) {
        ui.next_size(0, 44);
        let was = self.loc_on;
        ui.switch(ID_LOC, "Standort verwenden", &mut self.loc_on);
        if self.loc_on != was {
            self.location = (self.loc_on != 0).then(ps::location_watch);
        }
        let st = ps::location_state();
        let (text, hint) = match st {
            LocationState::Off => ("aus", ""),
            LocationState::Searching => ("sucht", "Draußen geht es schneller."),
            LocationState::Fix => ("Position da", ""),
            LocationState::Paused => ("pausiert", "Pausiert, solange die App im Hintergrund ist."),
            LocationState::Denied => ("nicht erlaubt", "Die App darf den Standort nicht abfragen: in den Einstellungen des Systems erlauben und den Schalter neu setzen."),
            LocationState::Disabled => ("Standort des Systems ist aus", "In den Einstellungen des Systems einschalten."),
            LocationState::Unavailable => ("nicht verfügbar", "Dieses Gerät hat keinen Standortdienst."),
        };
        ui.text(TS_SUBTITLE, &format!("Standort: {text}"));
        if !hint.is_empty() {
            ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, hint);
        }
        let fix = ps::location_fix();
        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 8);
        let dash = || String::from("–");
        let lat = fix.map_or(dash(), |f| format!("{}°", num(ui, f.latitude, 6)));
        let lon = fix.map_or(dash(), |f| format!("{}°", num(ui, f.longitude, 6)));
        let acc = fix.and_then(|f| f.accuracy).map_or(dash(), |v| format!("{} m", num(ui, v as f64, 0)));
        let alt = fix.and_then(|f| f.altitude).map_or(dash(), |v| format!("{} m", num(ui, v, 0)));
        let speed = fix.and_then(|f| f.speed).map_or(dash(), |v| format!("{} m/s ({} km/h)", num(ui, v as f64, 1), num(ui, v as f64 * 3.6, 1)));
        let bearing = fix.and_then(|f| f.bearing).map_or(dash(), |v| format!("{}°", num(ui, v as f64, 0)));
        let age = fix.map_or(dash(), |f| format!("{} s", f.age_ms() / 1000));
        let source = fix.map_or(dash(), |f| {
            String::from(match f.source {
                FixSource::Gps => "Satelliten",
                FixSource::Network => "Netzwerk",
                FixSource::Other => "keine Angabe",
            })
        });
        ui.text(TS_BODY, &format!("Breite: {lat}"));
        ui.text(TS_BODY, &format!("Länge: {lon}"));
        ui.text(TS_BODY, &format!("Genauigkeit: {acc}"));
        ui.text(TS_BODY, &format!("Höhe: {alt}"));
        ui.text(TS_BODY, &format!("Tempo: {speed}"));
        ui.text(TS_BODY, &format!("Richtung: {bearing}"));
        ui.text(TS_BODY, &format!("Alter: {age}"));
        ui.text(TS_BODY, &format!("Quelle: {source}"));
        ui.box_end();
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, "Keine Karte, kein Netzwerkzugriff der App: nur die Werte des Systems.");
        if fix.is_some() {
            ui.timer(TIMER_AGE, 1000); // the age goes on
        }
        self.save(ui);
    }

    // (self-test) data://sensors-state.txt: at once when a state changed, else a few times a second
    fn save(&mut self, ui: &Ui) {
        if !*self.selftest.get_or_insert_with(|| plat::env("jfgui_SELFTEST").as_deref() == Some("sensors")) {
            return;
        }
        let mut key = format!("selftest=sensors\npage={:?}\nsuspend={}\nloc.on={}\n", self.page, self.suspend, self.loc_on);
        for (bit, _, _, name) in SENSORS {
            key.push_str(&format!("{name}.state={}\n", state_key(ps::state(bit))));
        }
        let loc = ps::location_state();
        key.push_str(&format!("loc.state={}\n", match loc {
            LocationState::Off => "off",
            LocationState::Searching => "searching",
            LocationState::Fix => "fix",
            LocationState::Paused => "paused",
            LocationState::Denied => "denied",
            LocationState::Disabled => "disabled",
            LocationState::Unavailable => "unavailable",
        }));
        let now = ui.now();
        if key == self.written_key && now.wrapping_sub(self.written_at) < 200 {
            return;
        }
        let mut text = key.clone();
        text.push_str(&format!("uic_sensor={}\nuic_location={}\n", self.uic_sensor, self.uic_location));
        for (bit, _, _, name) in SENSORS {
            if let Some(r) = ps::read(bit) {
                text.push_str(&format!("{name}={} {} {} {}\n{name}.accuracy={}\n", r.v[0], r.v[1], r.v[2], r.v[3], r.accuracy));
            }
        }
        if let Some(Attitude { pitch, roll, side, heading, absolute, .. }) = ps::attitude() {
            text.push_str(&format!("pitch={pitch}\nroll={roll}\nside={side}\nabsolute={}\n", absolute as u32));
            if let Some(h) = heading {
                text.push_str(&format!("heading={h}\n"));
            }
        }
        if let Some(f) = ps::location_fix() {
            text.push_str(&format!("lat={}\nlon={}\nage={}\nsource={:?}\n", f.latitude, f.longitude, f.age_ms(), f.source));
            if let Some(v) = f.accuracy {
                text.push_str(&format!("accuracy={v}\n"));
            }
            if let Some(v) = f.altitude {
                text.push_str(&format!("altitude={v}\n"));
            }
            if let Some(v) = f.speed {
                text.push_str(&format!("speed={v}\n"));
            }
            if let Some(v) = f.bearing {
                text.push_str(&format!("bearing={v}\n"));
            }
        }
        if text != self.written {
            plat::write_file(STATE, text.as_bytes());
            self.written = text;
        }
        self.written_key = key;
        self.written_at = now;
    }
}
