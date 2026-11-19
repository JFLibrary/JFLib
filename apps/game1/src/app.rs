// app -- game 1's window: a ui.gpu_view over the whole window (scene.rs) with the HUD and the menus drawn over it
//
//   start menu      name, server (desktop; the browser plays on the server the page came from), room code; create a
//                   room, join one by its code, practice against bots in this process, settings
//   playing         a click into the view locks the pointer (that click fires nothing: game::Input); the HUD: health,
//                   ammo, crosshair, hit marker, kill feed, the room's code; Tab holds the scoreboard
//   pause           the lock ended (Esc is the lock's, docs/GAME.md G7a): continue, settings, leave the room
//   settings        sensitivity, field of view, invert; kept with lib/game's ActionMap/MouseSettings (Input::store)
//                   and the game's own keys in config://settings.ini
// Without a room the view shows the map from a slowly circling camera.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use core::cell::RefCell;

use base::settings::{Schema, Settings};
use game::input::{Input, MouseSettings};
use game::net::client::Phase;
use game::Vec3;
use game1_core::map::Map;
use game1_core::rules::{GAME_NAME, HEALTH, MAGAZINE, WS_PATH};
use ui::*;

use crate::play::{Controls, Play};
use crate::scene::{Frame, View};

const ID_VIEW: u32 = 100;
const ID_NAME: u32 = 110;
const ID_SERVER: u32 = 111;
const ID_CODE: u32 = 112;
const ID_CREATE: u32 = 120;
const ID_JOIN: u32 = 121;
const ID_PRACTICE: u32 = 122;
const ID_SETTINGS: u32 = 123;
const ID_QUIT: u32 = 124;
const ID_CONTINUE: u32 = 125;
const ID_LEAVE: u32 = 126;
const ID_BACK: u32 = 127;
const ID_SENS: u32 = 130;
const ID_FOV: u32 = 131;
const ID_INV_Y: u32 = 132;
const ID_INV_X: u32 = 133;
const ID_PANEL: u32 = 140;
pub const TIMER_FRAME: u32 = 1;
pub const CMD_QUIT: u32 = 1;

static SCHEMA: Schema = Schema::new(1, &[]);
const SETTINGS: &str = "config://settings.ini";

/// bots in the practice mode
const PRACTICE_BOTS: usize = 7;

/// the game's window (see the module header)
pub struct Game1 {
    view: Rc<RefCell<View>>,
    ver: u32,
    map: Arc<Map>,
    input: Input,
    controls: Controls,
    settings: Settings,
    play: Option<Play>,
    settings_open: bool,
    name: String,
    server: String,
    code: String,
    sens: i32,
    fov: i32,
    inv_y: i32,
    inv_x: i32,
    message: String,
    orbit: f64,
    last_us: u64,
}

impl Game1 {
    pub fn new(map: Arc<Map>) -> Game1 {
        let (amap, controls) = Controls::map();
        let mut input = Input::new(amap);
        // (a scripted run never reads or writes the person's settings)
        let scripted = plat::args().iter().any(|a| a == "--script");
        let settings = Settings::open(if scripted { "mem://game1/settings.ini" } else { SETTINGS }, &SCHEMA);
        input.load(settings.ini(), "input");
        let ini = settings.ini();
        let name = String::from(ini.get("game1", "name").unwrap_or("Spieler"));
        let server = String::from(ini.get("game1", "server").unwrap_or(""));
        let fov = ini.get_int("game1", "fov", 100).clamp(60, 130) as i32;
        let m = *input.mouse();
        let view = Rc::new(RefCell::new(View::new(map.clone())));
        Game1 {
            view,
            ver: 0,
            map,
            input,
            controls,
            settings,
            play: None,
            settings_open: false,
            name,
            server,
            code: String::new(),
            sens: (m.sensitivity * 100.0) as i32,
            fov,
            inv_y: m.invert_y as i32,
            inv_x: m.invert_x as i32,
            message: String::new(),
            orbit: 0.0,
            last_us: 0,
        }
    }

    fn save(&mut self) {
        let mut m: MouseSettings = *self.input.mouse();
        m.sensitivity = self.sens as f64 / 100.0;
        m.invert_y = self.inv_y != 0;
        m.invert_x = self.inv_x != 0;
        self.input.set_mouse(m);
        let ini = self.settings.ini_mut();
        self.input.store(ini, "input");
        ini.set("game1", "name", &self.name);
        ini.set("game1", "server", &self.server);
        ini.set_int("game1", "fov", self.fov as i64);
        self.settings.save();
    }

    /// the vertical field of view from the setting (the setting is the horizontal one at 16:9, as games count it)
    fn fov_y(&self, aspect: f64) -> f64 {
        let h = self.fov as f64 * core::f64::consts::PI / 180.0;
        let t = physics::math::sin(h * 0.5) / physics::math::cos(h * 0.5);
        let _ = aspect;
        2.0 * physics::math::atan2(t / (16.0 / 9.0), 1.0)
    }

    fn start(&mut self, ui: &mut Ui, how: u32) {
        self.message.clear();
        let name = game::net::room::clean_name(&self.name).unwrap_or_else(|| String::from("Spieler"));
        self.name = name.clone();
        self.save();
        let room = if how == ID_JOIN {
            match game::net::room::room_from_link(&self.code) {
                Some(c) => Some(c),
                None => {
                    self.message = String::from("Der Raumcode hat sechs Zeichen (ohne 0, O, 1, I, L).");
                    return;
                }
            }
        } else {
            None
        };
        let play = if how == ID_PRACTICE {
            Ok(Play::practice(&name, PRACTICE_BOTS, self.map.clone(), plat::random_u64()))
        } else {
            match endpoint(&self.server) {
                Ok(ep) => Play::online(ep, &name, room, self.map.clone()),
                Err(e) => Err(e),
            }
        };
        match play {
            Ok(p) => {
                self.play = Some(p);
                self.input.release_all();
                ui.pointer_lock(true);
                ui.focus(ID_VIEW);
            }
            Err(e) => self.message = format!("Keine Verbindung: {}", e),
        }
    }

    fn leave(&mut self, ui: &mut Ui) {
        if let Some(mut p) = self.play.take() {
            p.close();
        }
        ui.pointer_lock(false);
        self.settings_open = false;
    }

    /// the panel of the menus: a box in the middle of the window over a dark plate
    fn panel(ui: &mut Ui, w: i32, h: i32, pw: i32, ph: i32) {
        let (x, y) = ((w - pw) / 2, (h - ph) / 2);
        ui.draw_color(0xE61C_2230);
        ui.draw_rrect(x - 16, y - 16, pw + 32, ph + 32, 14);
        ui.next_rect_px(x, y, pw, ph);
        ui.box_begin(ID_PANEL, BOX_V, 8);
    }

    fn menu_start(&mut self, ui: &mut Ui, w: i32, h: i32) {
        let web = cfg!(target_arch = "wasm32");
        Game1::panel(ui, w, h, 380, if web { 400 } else { 470 });
        ui.heading(1, TS_TITLE, GAME_NAME);
        ui.text(TS_BODY | TX_SECONDARY, "Ein schneller Cartoon-Shooter, jeder gegen jeden.");
        ui.spacer(6);
        ui.text(TS_CAPTION, "Name");
        ui.next_size(-1, 0);
        ui.textbox(ID_NAME, Some("Spieler"), &mut self.name, 0);
        if !web {
            ui.text(TS_CAPTION, "Server (Adresse:Port)");
            ui.next_size(-1, 0);
            ui.textbox(ID_SERVER, Some("127.0.0.1"), &mut self.server, 0);
        }
        ui.text(TS_CAPTION, "Raumcode zum Beitreten");
        ui.next_size(-1, 0);
        ui.textbox(ID_CODE, Some("z. B. K7QX3M"), &mut self.code, 0);
        ui.spacer(6);
        ui.next_size(-1, 0);
        if ui.button(ID_CREATE, "Raum erstellen") & EV_ACT != 0 {
            self.start(ui, ID_CREATE);
        }
        ui.next_size(-1, 0);
        if ui.button(ID_JOIN, "Raum beitreten") & EV_ACT != 0 {
            self.start(ui, ID_JOIN);
        }
        ui.next_size(-1, 0);
        if ui.button(ID_PRACTICE, "Üben gegen Bots") & EV_ACT != 0 {
            self.start(ui, ID_PRACTICE);
        }
        ui.next_size(-1, 0);
        if ui.button(ID_SETTINGS, "Einstellungen") & EV_ACT != 0 {
            self.settings_open = true;
        }
        if !web {
            ui.next_size(-1, 0);
            if ui.button(ID_QUIT, "Beenden") & EV_ACT != 0 {
                ui.quit();
            }
        }
        if !self.message.is_empty() {
            ui.text(TS_CAPTION, &self.message);
        }
        ui.box_end();
    }

    fn menu_pause(&mut self, ui: &mut Ui, w: i32, h: i32) {
        Game1::panel(ui, w, h, 340, 230);
        ui.heading(1, TS_TITLE, "Pause");
        let room = self.play.as_ref().map(|p| p.hud.room.clone()).unwrap_or_default();
        if !room.is_empty() {
            ui.text(TS_BODY, &format!("Raum {} – den Code weitergeben, um gemeinsam zu spielen", room));
        }
        ui.spacer(6);
        ui.next_size(-1, 0);
        if ui.button(ID_CONTINUE, "Weiterspielen") & EV_ACT != 0 {
            ui.pointer_lock(true);
            ui.focus(ID_VIEW);
        }
        ui.next_size(-1, 0);
        if ui.button(ID_SETTINGS, "Einstellungen") & EV_ACT != 0 {
            self.settings_open = true;
        }
        ui.next_size(-1, 0);
        if ui.button(ID_LEAVE, "Raum verlassen") & EV_ACT != 0 {
            self.leave(ui);
        }
        ui.box_end();
    }

    fn menu_settings(&mut self, ui: &mut Ui, w: i32, h: i32) {
        Game1::panel(ui, w, h, 380, 330);
        ui.heading(1, TS_TITLE, "Einstellungen");
        ui.text(TS_CAPTION, &format!("Empfindlichkeit der Maus: {:.2}", self.sens as f64 / 100.0));
        ui.next_size(-1, 0);
        ui.slider(ID_SENS, &mut self.sens, &Range::new(5, 1000, 5, 0));
        ui.text(TS_CAPTION, &format!("Sichtfeld: {}°", self.fov));
        ui.next_size(-1, 0);
        ui.slider(ID_FOV, &mut self.fov, &Range::new(70, 120, 1, 0));
        ui.checkbox(ID_INV_Y, "Maus oben/unten umkehren", &mut self.inv_y);
        ui.checkbox(ID_INV_X, "Maus links/rechts umkehren", &mut self.inv_x);
        ui.spacer(6);
        ui.next_size(-1, 0);
        if ui.button(ID_BACK, "Zurück") & EV_ACT != 0 {
            self.settings_open = false;
            self.save();
        }
        ui.box_end();
        // (the values take effect at once)
        let mut m = *self.input.mouse();
        m.sensitivity = self.sens as f64 / 100.0;
        m.invert_y = self.inv_y != 0;
        m.invert_x = self.inv_x != 0;
        self.input.set_mouse(m);
    }
}

/// the server's address -> an endpoint: "host:port" or "host" (DEFAULT_PORT); UDP on the desktop, the WebSocket of
/// the page's own server in the browser
fn endpoint(server: &str) -> Result<netchan::Endpoint, String> {
    let s = server.trim();
    if cfg!(target_arch = "wasm32") {
        return Ok(netchan::Endpoint::WebSocket(if s.is_empty() { String::from(WS_PATH) } else { format!("ws://{}{}", s, WS_PATH) }));
    }
    #[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
    {
        let s = if s.is_empty() { "127.0.0.1" } else { s };
        if let Some(a) = plat::net::SocketAddr::parse(s) {
            return Ok(netchan::Endpoint::Udp(a));
        }
        let (host, port) = match s.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') || h.ends_with(']') => (h, p.parse::<u16>().map_err(|_| format!("{} ist kein Port", p))?),
            _ => (s, game1_core::rules::DEFAULT_PORT),
        };
        let addrs = plat::net::resolve(host, port).map_err(|e| format!("{}: {}", host, e))?;
        addrs.into_iter().next().map(netchan::Endpoint::Udp).ok_or_else(|| format!("{}: keine Adresse", host))
    }
    #[cfg(not(any(windows, target_os = "linux", target_vendor = "apple")))]
    {
        Err(String::from("auf diesem Gerät gibt es kein Netz"))
    }
}

/// a colour with alpha
fn argb(a: u32, rgb: u32) -> u32 {
    (a << 24) | (rgb & 0x00ff_ffff)
}

fn text(ui: &mut Ui, x: i32, y: i32, s: &str, px: i32, bold: bool, color: u32) {
    // (a shadow under the text keeps it readable on every background)
    ui.draw_color(0xA000_0000);
    ui.draw_text(x + 1, y + 1, s, ((px as u32) << 16) | if bold { FONT_BOLD } else { FONT_REG });
    ui.draw_color(color);
    ui.draw_text(x, y, s, ((px as u32) << 16) | if bold { FONT_BOLD } else { FONT_REG });
}

/// an estimate of a text's width (the HUD aligns right)
fn text_w(s: &str, px: i32) -> i32 {
    (s.chars().count() as f64 * px as f64 * 0.56) as i32
}

impl Game1 {
    fn hud(&self, ui: &mut Ui, w: i32, h: i32, scores: bool) {
        let Some(p) = &self.play else { return };
        let k = (h as f64 / 900.0).clamp(0.7, 2.0);
        let s = |v: f64| (v * k) as i32;
        let hud = &p.hud;
        let (cx, cy) = (w / 2, h / 2);
        let fx = |v: i32| v << 8; // 24.8 for lines
        match hud.phase {
            Some(Phase::Playing) => {}
            Some(Phase::Refused(r)) => {
                text(ui, cx - s(220.0), cy, &format!("Der Server hat abgelehnt ({}): Raum voll oder unbekannt.", r), s(20.0), true, 0xFFFF_FFFF);
                return;
            }
            Some(Phase::Closed) => {
                text(ui, cx - s(160.0), cy, "Die Verbindung ist zu Ende.", s(22.0), true, 0xFFFF_FFFF);
                return;
            }
            _ => {
                text(ui, cx - s(80.0), cy, "Verbinde …", s(24.0), true, 0xFFFF_FFFF);
                return;
            }
        }
        // damage: a red frame
        if hud.hurt > 0.0 {
            let a = (hud.hurt / 0.35 * 120.0) as u32;
            ui.draw_color(argb(a, 0xd0_2020));
            let t = s(26.0);
            ui.draw_rect(0, 0, w, t);
            ui.draw_rect(0, h - t, w, t);
            ui.draw_rect(0, 0, t, h);
            ui.draw_rect(w - t, 0, t, h);
        }
        if hud.dead {
            let line = if hud.killer.is_empty() { String::from("Du bist raus – gleich geht's weiter") } else { format!("{} hat dich erwischt – gleich geht's weiter", hud.killer) };
            text(ui, cx - text_w(&line, s(24.0)) / 2, cy - s(40.0), &line, s(24.0), true, 0xFFFF_E080);
        } else {
            // the crosshair: four short lines and a dot
            let (g, l, t) = (s(5.0).max(3), s(9.0).max(5), s(2.0).max(1) as i32);
            ui.draw_color(0xF0FF_FFFF);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                ui.draw_line(fx(cx + dx * g), fx(cy + dy * g), fx(cx + dx * (g + l)), fx(cy + dy * (g + l)), fx(t));
            }
            ui.draw_circle(fx(cx), fx(cy), fx(t.max(2)), 0);
            // the hit marker: an X, red for a kill, yellow for a head shot
            if hud.hit > 0.0 {
                let c = if hud.hit_kill { 0xFFFF_3030 } else if hud.hit_head { 0xFFFF_D030 } else { 0xFFFF_FFFF };
                ui.draw_color(c);
                let (a, b) = (s(8.0), s(16.0));
                for (dx, dy) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                    ui.draw_line(fx(cx + dx * a), fx(cy + dy * a), fx(cx + dx * b), fx(cy + dy * b), fx(s(2.5).max(2)));
                }
            }
        }
        // health (bottom left) and ammo (bottom right)
        let pad = s(28.0);
        let hp = hud.health.clamp(0, HEALTH);
        let bar_w = s(220.0);
        ui.draw_color(0x8010_1418);
        ui.draw_rrect(pad, h - pad - s(20.0), bar_w, s(20.0), s(6.0));
        ui.draw_color(if hp > 30 { 0xFF4C_D964 } else { 0xFFFF_5040 });
        ui.draw_rrect(pad, h - pad - s(20.0), bar_w * hp / HEALTH, s(20.0), s(6.0));
        text(ui, pad, h - pad - s(30.0), &format!("{}", hp), s(34.0), true, 0xFFFF_FFFF);
        let ammo = if hud.reloading { String::from("lädt …") } else { format!("{} / {}", hud.ammo, MAGAZINE) };
        let col = if hud.ammo <= 4 && !hud.reloading { 0xFFFF_7050 } else { 0xFFFF_FFFF };
        text(ui, w - pad - text_w(&ammo, s(34.0)), h - pad, &ammo, s(34.0), true, col);
        if hud.protected {
            text(ui, cx - s(70.0), h - pad, "geschützt", s(18.0), false, 0xFFA0_E0FF);
        }
        // the room (top left), the kill feed (top right)
        if !hud.room.is_empty() {
            text(ui, pad, pad + s(10.0), &format!("Raum {}", hud.room), s(18.0), true, 0xFFFF_FFFF);
        }
        if let Some(ping) = hud.ping_ms {
            text(ui, pad, pad + s(32.0), &format!("{:.0} ms", ping), s(14.0), false, 0xC0FF_FFFF);
        }
        let mut y = pad + s(10.0);
        for l in &hud.feed {
            let line = if l.killer.is_empty() { format!("{} ist abgestürzt", l.victim) } else if l.head { format!("{}  ⊗ Kopf  {}", l.killer, l.victim) } else { format!("{}  ⊗  {}", l.killer, l.victim) };
            let px = s(16.0);
            let tw = text_w(&line, px);
            ui.draw_color(if l.mine { 0x90A0_3010 } else { 0x7010_1418 });
            ui.draw_rrect(w - pad - tw - s(10.0), y - px - s(2.0), tw + s(20.0), px + s(10.0), s(5.0));
            text(ui, w - pad - tw, y, &line, px, false, 0xFFFF_FFFF);
            y += px + s(14.0);
        }
        // the scoreboard (Tab held)
        if scores {
            let rows = p.scores();
            let (bw, rh) = (s(460.0), s(30.0));
            let bh = rh * (rows.len() as i32 + 2);
            let (bx, by) = (cx - bw / 2, cy - bh / 2);
            ui.draw_color(0xE01A_1E28);
            ui.draw_rrect(bx, by, bw, bh, s(10.0));
            text(ui, bx + s(18.0), by + rh, "Name", s(16.0), true, 0xFFB0_B8C8);
            text(ui, bx + bw - s(170.0), by + rh, "Treffer", s(16.0), true, 0xFFB0_B8C8);
            text(ui, bx + bw - s(80.0), by + rh, "Tode", s(16.0), true, 0xFFB0_B8C8);
            for (i, r) in rows.iter().enumerate() {
                let y = by + rh * (i as i32 + 2);
                let c = if r.me { 0xFFFF_E080 } else { 0xFFFF_FFFF };
                text(ui, bx + s(18.0), y, &r.name, s(17.0), r.me, c);
                text(ui, bx + bw - s(150.0), y, &format!("{}", r.kills), s(17.0), r.me, c);
                text(ui, bx + bw - s(64.0), y, &format!("{}", r.deaths), s(17.0), r.me, c);
            }
        }
    }
}

impl App for Game1 {
    fn frame(&mut self, ui: &mut Ui) {
        let (w, h) = (ui.fb().w, ui.fb().h);
        ui.next_rect_px(0, 0, w, h);
        ui.next_flags(HB_KEYS);
        ui.canvas_label("Spiel");
        let inp = ui.gpu_view(ID_VIEW, &self.view, self.ver).clone();
        let now_us = plat::task::now_us();
        let dt = if self.last_us == 0 { 0.0 } else { now_us.saturating_sub(self.last_us) as f64 * 1e-6 };
        self.last_us = now_us;
        let in_room = self.play.is_some();
        let menu = !in_room || self.settings_open;
        // the input: keys while the view has the focus, the mouse while the game plays
        self.input.set_lock(ui.pointer_lock_state());
        if inp.ev & EV_FOCUS != 0 {
            while let Some(k) = ui.key_event_take() {
                self.input.key(k.code, k.down);
            }
        }
        if in_room && !menu {
            self.input.buttons(inp.pressed, inp.released);
        }
        let (dx, dy) = ui.pointer_motion();
        self.input.motion(dx, dy);
        if self.input.take_lock_request() && in_room && !menu {
            ui.pointer_lock(true);
        }
        self.input.take_pause();
        let playing = self.input.playing() && in_room && !menu;
        let (lx, ly) = (self.input.motion_of(self.controls.look_x), self.input.motion_of(self.controls.look_y));
        let scores = playing && self.input.down(self.controls.scores);
        let fov = self.fov_y(w as f64 / h.max(1) as f64);
        {
            let mut v = self.view.borrow_mut();
            match &mut self.play {
                Some(p) => {
                    if playing {
                        p.look(lx, ly);
                    }
                    let f = p.frame(now_us.saturating_mul(1000), &mut self.input, playing, fov);
                    v.frame = f;
                    let fx: alloc::vec::Vec<_> = p.fx.drain(..).collect();
                    v.fx.extend(fx);
                }
                None => {
                    // the map from a slowly circling camera
                    self.orbit += dt * 0.08;
                    let (sn, cs) = physics::math::sincos(self.orbit);
                    let eye = Vec3::new(30.0 * sn, 11.0, 30.0 * cs);
                    let yaw = crate::look_yaw(-eye.x, -eye.z);
                    v.frame = Frame { eye, yaw, pitch: -0.32, fov, figures: alloc::vec::Vec::new(), show_vm: false, time: v.frame.time + dt, ..Frame::default() };
                }
            }
        }
        self.input.end_frame();
        self.ver = self.ver.wrapping_add(1);
        ui.timer(TIMER_FRAME, 1);
        if in_room {
            self.hud(ui, w, h, scores);
        }
        if self.settings_open {
            self.menu_settings(ui, w, h);
        } else if !in_room {
            self.menu_start(ui, w, h);
        } else if !self.input.playing() {
            self.menu_pause(ui, w, h);
        }
        // a server that refused or ended: back to the start
        if let Some(p) = &self.play {
            if let Some(Phase::Refused(_) | Phase::Closed) = p.hud.phase {
                if !self.input.playing() && self.message.is_empty() {
                    self.message = String::from(match p.hud.phase {
                        Some(Phase::Refused(_)) => "Der Server hat abgelehnt: Raum voll, unbekannt oder eine andere Version.",
                        _ => "Die Verbindung ist zu Ende.",
                    });
                    self.play = None;
                }
            }
        }
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            CMD_QUIT => ui.quit(),
            UIC_TIMER if arg == TIMER_FRAME => ui.invalidate(),
            _ => return false,
        }
        true
    }
}
