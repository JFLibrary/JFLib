// play -- a client of game 1 while it plays: the connection (the practice mode in this process, or a server over
// UDP or WebSocket), the controls, the ticks (game::Input's tick -> NetInput -> NetClient::step: prediction), what
// the frame shows (the own figure predicted and blended by the clock's alpha, the others interpolated) and what the
// HUD needs (health, ammo, the hit marker, the kill feed, the scores); effects for the scene (muzzle flashes, tracers,
// impacts, puffs) from the own predicted shots and the others' shot counters.  The kill feed and the hit marker come
// from the room's events (game::net's reliable events: each once); the own figure is drawn with the decaying rest of
// a correction (NetClient::smoothing); leaving sends LEAVE (the server lets the figure go at once)
//
//   Controls        the actions (ActionMap) and their defaults: WASD by place, Space, Ctrl, Shift, the mouse, R, Tab
//   Conn            Local (practice: a server with bots on the simulated network) or Online (NetLink)
//   Play::frame     one frame: the ticks due with the input, the scene's Frame and effects, the HUD's state

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use game::input::{ActionId, ActionKind, ActionMap, Bind, Input, Source, MOUSE_LEFT};
use game::net::client::{lerp_angle_field, lerp_field, NetClient, Phase, Remote};
use game::net::input::NetInput;
use game::net::link::NetLink;
use game::net::quant::{angle_f, angle_q, axis_q, pitch_f, pitch_q, POS_STEPS};
use game::net::server::ServerOptions;
use game::net::RoomCode;
use game::Vec3;
use game1_core::map::Map;
use game1_core::movement::{MoveState, Mover, F_CROUCH, F_DEAD, F_GROUND};
use game1_core::rules::*;
use game1_core::sim::MatchConfig;
use physics::Filter;
use plat::{KC_A, KC_C, KC_CONTROL_LEFT, KC_D, KC_R, KC_S, KC_SHIFT_LEFT, KC_SPACE, KC_TAB, KC_W};

use crate::local::{Local, LOCAL};
use crate::look::{self, Group, Pose, VmState, PALETTE};
use crate::scene::{Frame, Fx};

/// the actions of game 1
#[derive(Clone, Copy, Debug)]
pub struct Controls {
    pub fwd: ActionId,
    pub side: ActionId,
    pub jump: ActionId,
    pub crouch: ActionId,
    pub walk: ActionId,
    pub fire: ActionId,
    pub reload: ActionId,
    pub scores: ActionId,
    pub look_x: ActionId,
    pub look_y: ActionId,
}

impl Controls {
    /// the action map with its defaults -> (map, ids)
    pub fn map() -> (ActionMap, Controls) {
        let mut m = ActionMap::new();
        let fwd = m.add("forward", ActionKind::Axis, &[Bind::key(KC_W), Bind::scaled(Source::Key(KC_S), -1.0)]);
        let side = m.add("strafe", ActionKind::Axis, &[Bind::key(KC_D), Bind::scaled(Source::Key(KC_A), -1.0)]);
        let jump = m.add("jump", ActionKind::Button, &[Bind::key(KC_SPACE)]);
        let crouch = m.add("crouch", ActionKind::Button, &[Bind::key(KC_CONTROL_LEFT), Bind::key(KC_C)]);
        let walk = m.add("walk", ActionKind::Button, &[Bind::key(KC_SHIFT_LEFT)]);
        let fire = m.add("fire", ActionKind::Button, &[Bind::new(Source::Mouse(MOUSE_LEFT))]);
        let reload = m.add("reload", ActionKind::Button, &[Bind::key(KC_R)]);
        let scores = m.add("scores", ActionKind::Button, &[Bind::key(KC_TAB)]);
        let look_x = m.add("look_x", ActionKind::Motion, &[Bind::new(Source::MouseX)]);
        let look_y = m.add("look_y", ActionKind::Motion, &[Bind::new(Source::MouseY)]);
        (m, Controls { fwd, side, jump, crouch, walk, fire, reload, scores, look_x, look_y })
    }

    /// the NetInput of a tick from the input's tick and the view
    pub fn net_input(&self, t: &game::TickInput, yaw: f64, pitch: f64) -> NetInput {
        let mut b = 0u16;
        for (a, bit) in [(self.jump, BTN_JUMP), (self.crouch, BTN_CROUCH), (self.walk, BTN_WALK), (self.fire, BTN_FIRE), (self.reload, BTN_RELOAD)] {
            if t.down(a) {
                b |= bit;
            }
        }
        NetInput { buttons: b, move_x: axis_q(t.value(self.side)), move_z: axis_q(t.value(self.fwd)), yaw: angle_q(yaw), pitch: pitch_q(pitch), view: 0 }
    }
}

/// how a client is connected
pub enum Conn {
    /// the practice mode: a server with bots in this process
    Local(Box<Local>),
    /// a server over the network
    Online(Box<NetLink<Mover>>),
}

/// a line of the kill feed
#[derive(Clone, Debug)]
pub struct FeedLine {
    pub at: f64,
    pub killer: String,
    pub victim: String,
    pub head: bool,
    pub mine: bool,
    /// a line that is no kill (a player came or went): shown as it is
    pub note: String,
}

/// a row of the scoreboard
#[derive(Clone, Debug)]
pub struct Score {
    pub id: u16,
    pub name: String,
    pub kills: i32,
    pub deaths: i32,
    pub me: bool,
}

/// what the HUD shows
#[derive(Clone, Debug, Default)]
pub struct Hud {
    pub health: i32,
    pub ammo: u32,
    pub reloading: bool,
    pub dead: bool,
    pub killer: String,
    pub protected: bool,
    /// seconds left of the hit marker (and whether it was a kill or a head shot)
    pub hit: f64,
    pub hit_kill: bool,
    pub hit_head: bool,
    /// seconds left of the red flash after damage
    pub hurt: f64,
    pub feed: Vec<FeedLine>,
    pub room: String,
    pub phase: Option<Phase>,
    pub ping_ms: Option<f64>,
}

/// a client playing (see the module header)
pub struct Play {
    pub conn: Conn,
    pub map: Arc<Map>,
    pub controls: Controls,
    pub yaw: f64,
    pub pitch: f64,
    pub hud: Hud,
    pub names: BTreeMap<u16, String>,
    /// the effects since the last frame handed to the scene
    pub fx: Vec<Fx>,
    rem: Vec<Remote>,
    shots: BTreeMap<u16, u8>,
    deaths: BTreeMap<u16, i32>,
    healths: BTreeMap<u16, i32>,
    phases: BTreeMap<u16, (f64, Vec3)>,
    last_health: i32,
    feed: VecDeque<FeedLine>,
    vm: VmState,
    own_phase: f64,
    time: f64,
    last_ns: u64,
    pub ticks: u64,
    /// the yaw the server gave the figure when it came back (applied once)
    spawned: Option<i32>,
    /// the practice mode's time not yet run (us)
    carry_us: u64,
}

impl Play {
    /// the practice mode: a server with `bots` bots in this process, the player over a millisecond of network
    pub fn practice(name: &str, bots: usize, map: Arc<Map>, seed: u64) -> Play {
        let mut l = Local::new(seed, ServerOptions::default(), MatchConfig { bots, seed, ..MatchConfig::default() }, map.clone());
        l.add_client(name, false, LOCAL, 0);
        Play::with(Conn::Local(Box::new(l)), map)
    }

    /// a server over the network: UDP (desktop) or a WebSocket ("ws://host:port/game1", in the browser "/game1")
    pub fn online(ep: netchan::Endpoint, name: &str, room: Option<RoomCode>, map: Arc<Map>) -> Result<Play, String> {
        let opts = game::net::client::ClientOptions { name: String::from(name), room, ..Default::default() };
        let nc = NetClient::new(opts, Mover::default(), schema(), game1_core::sim::Match::empty_baseline());
        let cfg = netchan::ClientConfig { protocol: PROTOCOL, ..netchan::ClientConfig::default() };
        let link = NetLink::connect(ep, cfg, nc, WS_PATH).map_err(|e| alloc::format!("{}", e))?;
        Ok(Play::with(Conn::Online(Box::new(link)), map))
    }

    fn with(conn: Conn, map: Arc<Map>) -> Play {
        let (_, controls) = Controls::map();
        Play {
            conn,
            map,
            controls,
            yaw: 0.0,
            pitch: 0.0,
            hud: Hud::default(),
            names: BTreeMap::new(),
            fx: Vec::new(),
            rem: Vec::new(),
            shots: BTreeMap::new(),
            deaths: BTreeMap::new(),
            healths: BTreeMap::new(),
            phases: BTreeMap::new(),
            last_health: HEALTH,
            feed: VecDeque::new(),
            vm: VmState::default(),
            own_phase: 0.0,
            time: 0.0,
            last_ns: 0,
            ticks: 0,
            spawned: None,
            carry_us: 0,
        }
    }

    /// the netcode's client
    pub fn client(&self) -> &NetClient<Mover> {
        match &self.conn {
            Conn::Local(l) => &l.clients[0].client,
            Conn::Online(l) => &l.game,
        }
    }

    fn client_mut(&mut self) -> &mut NetClient<Mover> {
        match &mut self.conn {
            Conn::Local(l) => &mut l.clients[0].client,
            Conn::Online(l) => &mut l.game,
        }
    }

    /// the room's code (once in a room)
    pub fn room(&self) -> Option<RoomCode> {
        self.client().welcome().map(|w| w.room)
    }

    /// leave: LEAVE and the end of the connection (the server lets the figure go at once)
    pub fn close(&mut self) {
        if let Conn::Online(link) = &mut self.conn {
            link.leave();
        }
    }

    /// turn the view by mouse motion (radians; the frame's, not the tick's: the camera turns every frame)
    pub fn look(&mut self, dx: f64, dy: f64) {
        self.yaw = game::camera::wrap_angle(self.yaw - dx);
        self.pitch = (self.pitch - dy).clamp(-game::camera::PITCH_LIMIT, game::camera::PITCH_LIMIT);
    }

    /// one frame at now_ns: the ticks due (each takes input.tick()), then the scene's frame
    pub fn frame(&mut self, now_ns: u64, input: &mut Input, playing: bool, fov: f64) -> Frame {
        let dt = if self.last_ns == 0 { 0.0 } else { (now_ns.saturating_sub(self.last_ns)) as f64 * 1e-9 };
        self.last_ns = now_ns;
        self.time += dt.min(0.25);
        let (yaw, pitch, ctl) = (self.yaw, self.pitch, self.controls);
        let mut fired_ticks = 0u32;
        let make = |input: &mut Input| -> NetInput {
            let t = input.tick();
            if playing { ctl.net_input(&t, yaw, pitch) } else { NetInput { yaw: angle_q(yaw), pitch: pitch_q(pitch), ..NetInput::default() } }
        };
        match &mut self.conn {
            Conn::Local(l) => {
                self.carry_us = (self.carry_us + (dt * 1e6) as u64).min(250_000);
                let ms = self.carry_us / 1000;
                self.carry_us -= ms * 1000;
                for _ in 0..ms {
                    let stepped = l.step_ms(&mut |k, _c| if k == 0 { make(input) } else { NetInput::default() });
                    for (k, _) in stepped {
                        if k == 0 {
                            self.ticks += 1;
                            if l.clients[0].client.predicted().is_some_and(|s| s.fired()) {
                                fired_ticks += 1;
                            }
                        }
                    }
                }
            }
            Conn::Online(link) => {
                link.pump(&self.map.world, now_ns);
                let n = link.game.advance(now_ns);
                for _ in 0..n {
                    let i = make(input);
                    link.game.step(&self.map.world, i);
                    self.ticks += 1;
                    if link.game.predicted().is_some_and(|s| s.fired()) {
                        fired_ticks += 1;
                    }
                }
                link.flush();
                self.hud.ping_ms = Some(link.channel().rtt_ms() as f64);
            }
        }
        self.after_ticks(fired_ticks, dt, fov)
    }

    fn own(&self) -> Option<(MoveState, MoveState, f64)> {
        let c = self.client();
        let cur = c.predicted()?;
        let prev = c.predicted_prev().unwrap_or(cur);
        Some((prev, cur, c.alpha()))
    }

    fn name_of(&self, id: u16) -> String {
        self.names.get(&id).cloned().unwrap_or_else(|| alloc::format!("#{}", id))
    }

    fn after_ticks(&mut self, fired: u32, dt: f64, fov: f64) -> Frame {
        let mover = Mover::default();
        let own_id = self.client().own_id();
        // the others, interpolated, and the names
        let mut rem = core::mem::take(&mut self.rem);
        self.client().remotes(&mut rem);
        let mut figures = Vec::new();
        for r in &rem {
            if r.a.kind == KIND_TAG {
                let id = r.a.fields[0] as u16;
                if !self.names.contains_key(&id) {
                    self.names.insert(id, fields_name(&r.a.fields[1..]));
                }
                continue;
            }
            if r.a.kind != KIND_FIG {
                continue;
            }
            let f = |i: usize| lerp_field(r.a.fields[i], r.b.fields[i], r.t);
            let feet = Vec3::new(f(0) / POS_STEPS, f(1) / POS_STEPS, f(2) / POS_STEPS);
            let near = if r.t < 0.5 { &r.a } else { &r.b };
            let flags = near.fields[6] as u32;
            let yaw = angle_f(0) + lerp_angle_field(r.a.fields[F_YAW], r.b.fields[F_YAW], r.t) * core::f64::consts::TAU / 65536.0;
            let pitch = pitch_f(lerp_field(r.a.fields[F_PITCH], r.b.fields[F_PITCH], r.t) as i16);
            let vel = Vec3::new(f(3) / POS_STEPS, 0.0, f(5) / POS_STEPS);
            let (ph, _) = self.phases.entry(r.id).or_insert((0.0, feet));
            let speed = vel.length();
            *ph += speed * dt * 2.2;
            let pose = Pose { feet, yaw, pitch, crouched: flags & F_CROUCH != 0, grounded: flags & F_GROUND != 0, phase: *ph, swing: (speed / 6.0).min(1.0) };
            // shots: the counter in the weapon field went up
            let shots = (near.fields[7] as u32 >> 24) as u8;
            let last = *self.shots.entry(r.id).or_insert(shots);
            if shots != last && flags & F_DEAD == 0 {
                let muzzle = look::model_point(Group::Arms, &pose, self.muzzle_model());
                let (dir, _, _) = game::camera::basis(yaw, pitch);
                self.fx.push(Fx::Muzzle { at: muzzle, dir });
                let eye = feet + Vec3::new(0.0, mover.height(pose.crouched) - EYE_BELOW_TOP, 0.0);
                self.shot_fx(muzzle, eye, dir);
            }
            self.shots.insert(r.id, shots);
            // deaths: the counter went up -> a puff where the figure is seen (the feed's line is the event's)
            let deaths = near.fields[F_DEATHS];
            let last = *self.deaths.entry(r.id).or_insert(deaths);
            if deaths > last {
                self.fx.push(Fx::Poof { at: feet + Vec3::new(0.0, 0.9, 0.0), color: PALETTE[r.id as usize % PALETTE.len()] });
            }
            self.deaths.insert(r.id, deaths);
            // a hit: the health went down
            let health = near.fields[F_HEALTH];
            if self.healths.get(&r.id).is_some_and(|h| health < *h) && health > 0 {
                self.fx.push(Fx::Hit { at: feet + Vec3::new(0.0, if pose.crouched { 0.7 } else { 1.1 }, 0.0) });
            }
            self.healths.insert(r.id, health);
            if flags & F_DEAD == 0 {
                figures.push((r.id, pose));
            }
        }
        self.rem = rem;
        // the room's events: the kill feed, the hit marker, players coming and going
        while let Some(e) = self.client_mut().take_event() {
            match GameEvent::decode(e.channel, &e.data) {
                Some(GameEvent::Kill { killer, victim, head }) => {
                    self.push_feed(killer, victim, head, own_id);
                    if victim == own_id {
                        self.hud.killer = if killer == victim { String::new() } else { self.name_of(killer) };
                    }
                }
                Some(GameEvent::Hit { head, kill, .. }) => {
                    self.hud.hit = 0.25;
                    self.hud.hit_head = head;
                    self.hud.hit_kill = kill;
                }
                Some(GameEvent::Joined { id, name }) => {
                    self.push_note(alloc::format!("{} ist dabei", name));
                    self.names.insert(id, name);
                }
                Some(GameEvent::Left { name, .. }) => self.push_note(alloc::format!("{} hat den Raum verlassen", name)),
                None => {}
            }
        }
        // the own figure: between the last two ticks, with the decaying rest of a correction
        let newest = self.client().newest().and_then(|s| s.get(own_id)).map(|e| e.state);
        let (prev, cur, alpha) = self.own().unwrap_or_default();
        let a = prev.feet();
        let b = cur.feet();
        let feet = a + (b - a) * alpha + self.client().smoothing();
        let eye_h = mover.height(cur.crouched()) - EYE_BELOW_TOP;
        let prev_h = mover.height(prev.crouched()) - EYE_BELOW_TOP;
        let eye = feet + Vec3::new(0.0, prev_h + (eye_h - prev_h) * alpha, 0.0);
        if let Some(st) = newest {
            let f = &st.fields;
            self.hud.health = f[F_HEALTH];
            if f[F_HEALTH] < self.last_health && f[F_HEALTH] > 0 {
                self.hud.hurt = 0.35;
            }
            let deaths = f[F_DEATHS];
            self.last_health = f[F_HEALTH];
            self.hud.protected = f[F_INFO] & INFO_PROTECTED != 0;
            // came back: look the way the spawn point looks (once per life)
            if f[F_HEALTH] == HEALTH && cur.flags & F_DEAD == 0 && self.spawned != Some(deaths) {
                self.spawned = Some(deaths);
                self.yaw = info_spawn_yaw(f[F_INFO]);
                self.pitch = 0.0;
            }
        }
        self.hud.ammo = cur.ammo();
        self.hud.reloading = cur.reload() > 0;
        self.hud.dead = cur.dead();
        // the own shots: muzzle flash in front of the camera, a tracer and an impact
        if fired > 0 && !cur.dead() {
            self.vm.kick = 1.0;
            self.fx.push(Fx::MuzzleOwn);
            let (dir, right, up) = game::camera::basis(self.yaw, self.pitch);
            let from = eye + right * 0.2 - up * 0.15 + dir * 0.6;
            self.shot_fx(from, eye, dir);
        }
        // the viewmodel
        let speed = Vec3::new(cur.vel[0] as f64, 0.0, cur.vel[2] as f64).length() / POS_STEPS;
        self.own_phase += speed * dt * 1.6;
        self.vm.bob = self.own_phase;
        self.vm.bob_amount = if cur.grounded() { (speed / 9.0).min(1.0) } else { 0.2 };
        self.vm.kick = (self.vm.kick - dt * 9.0).max(0.0);
        let want = if cur.reload() > 0 { 1.0 } else { 0.0 };
        self.vm.reload += (want - self.vm.reload) * (dt * 10.0).min(1.0);
        // timers
        self.hud.hit = (self.hud.hit - dt).max(0.0);
        self.hud.hurt = (self.hud.hurt - dt).max(0.0);
        while self.feed.front().is_some_and(|l| self.time - l.at > 6.0) {
            self.feed.pop_front();
        }
        self.hud.feed = self.feed.iter().cloned().collect();
        self.hud.room = self.room().map(|r| String::from(r.as_str())).unwrap_or_default();
        self.hud.phase = Some(self.client().phase());
        Frame { eye, yaw: self.yaw, pitch: self.pitch, fov, figures, vm: self.vm, show_vm: !cur.dead() && self.client().predictor().known(), time: self.time }
    }

    fn muzzle_model(&self) -> Vec3 {
        Vec3::new(-0.12, 1.23, 0.96)
    }

    /// a tracer from `from` along the shot from `eye` in `dir` to where it meets the map, and the impact there
    fn shot_fx(&mut self, from: Vec3, eye: Vec3, dir: Vec3) {
        match self.map.world.ray_cast(eye, dir, RANGE, &Filter::ALL) {
            Some((_, hit)) => {
                let at = eye + dir * hit.t;
                self.fx.push(Fx::Tracer { from, to: at });
                self.fx.push(Fx::Impact { at, normal: hit.normal });
            }
            None => self.fx.push(Fx::Tracer { from, to: eye + dir * RANGE }),
        }
    }

    fn push_feed(&mut self, killer: u16, victim: u16, head: bool, own: u16) {
        let line = FeedLine { at: self.time, killer: if killer == victim { String::new() } else { self.name_of(killer) }, victim: self.name_of(victim), head, mine: killer == own || victim == own, note: String::new() };
        self.push_line(line);
    }

    fn push_note(&mut self, note: String) {
        self.push_line(FeedLine { at: self.time, killer: String::new(), victim: String::new(), head: false, mine: false, note });
    }

    fn push_line(&mut self, line: FeedLine) {
        self.feed.push_back(line);
        while self.feed.len() > 5 {
            self.feed.pop_front();
        }
    }

    /// the scoreboard: every figure of the newest snapshot, most kills first
    pub fn scores(&self) -> Vec<Score> {
        let own = self.client().own_id();
        let mut v = Vec::new();
        if let Some(s) = self.client().newest() {
            for e in &s.ents {
                if e.state.kind != KIND_FIG {
                    continue;
                }
                let f = &e.state.fields;
                v.push(Score { id: e.id, name: self.name_of(e.id), kills: f[F_KILLS], deaths: f[F_DEATHS], me: e.id == own });
            }
        }
        v.sort_by(|a, b| b.kills.cmp(&a.kills).then(a.deaths.cmp(&b.deaths)).then(a.id.cmp(&b.id)));
        v
    }
}
