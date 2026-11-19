// net_session -- a networked session in the process: game::net's NetServer with the arena and bots as
// NetClients, over game::net::sim's simulated network (netchan's cores on a virtual clock).  The groups
// netsim and bots, the checksum --netsum and the measurements --netbench run it
//
// Every millisecond of virtual time: the network delivers, the server takes its events, runs the ticks
// due (timed, for the CPU per tick), sends; every client takes its events, opens a fetch's WebSocket when
// WELCOME asks for it, runs its ticks (the bot makes each input from what the client sees) and sends.
// The harness keeps, for the checks, the server's state of every figure per tick, every client's first
// prediction per tick, and the world snapshots of the last ticks.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use game::net::arena::{self, Arena, ArenaConfig};
use game::net::bot::{remote_feet, Bot, BotConfig};
use game::net::client::{ClientOptions, NetClient, Phase, Remote};
use game::net::server::{NetServer, RoomSim, ServerOptions};
use game::net::sim::{SimNet, Weather, Which};
use game::net::snapshot::{tick_diff, Snapshot};
use game::net::walker::{WalkState, Walker};
use game::net::RoomCode;
use game::Vec3;
use netchan::endpoint::{ClientConfig, ClientEvent, ServerConfig, ServerEvent};

/// the protocol id of the tests' netchan
pub const PROTOCOL: u32 = 0x4737_0001;

/// a session's settings
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// the seed of the network, the server and the bots
    pub seed: u64,
    /// clients
    pub players: usize,
    /// every this many clients is a WebSocket client (0: none)
    pub ws_every: usize,
    /// the network of every client
    pub weather: Weather,
    /// the arena
    pub arena: ArenaConfig,
    /// the server
    pub server: ServerOptions,
    /// bots shoot
    pub fire: bool,
    /// the first client joins this room code instead of making one (the others join the first's room)
    pub room: Option<RoomCode>,
    /// netchan's limits per client (tests with many bots raise them)
    pub chan: ServerConfig,
    /// clients join one after another, this many ms apart
    pub join_gap_ms: u64,
    /// the first client joins after this many ms (the server runs before)
    pub join_after_ms: u64,
    /// time the server's ticks (plat::time_us; not in a checksum)
    pub time_ticks: bool,
}

impl SessionConfig {
    /// `players` bots in `weather`, a quarter of them over WebSocket, the arena without props
    pub fn new(seed: u64, players: usize, weather: Weather) -> SessionConfig {
        SessionConfig {
            seed,
            players,
            ws_every: 4,
            weather,
            arena: ArenaConfig { fire: false, ..ArenaConfig::default() },
            // (the fetch over a WebSocket, an option since 2026-11-19: the session covers it; netsim's "fill" the default)
            server: ServerOptions { ws_port: 7777, fetch_full: true, ..ServerOptions::default() },
            fire: false,
            room: None,
            chan: ServerConfig { protocol: PROTOCOL, ..ServerConfig::default() },
            join_gap_ms: 50,
            join_after_ms: 0,
            time_ticks: false,
        }
    }
}

/// one bot client of a session
pub struct Bc {
    /// its peer on the network
    pub peer: usize,
    /// the netcode's client
    pub client: NetClient<Walker>,
    /// its brain
    pub bot: Bot,
    /// over a WebSocket
    pub ws: bool,
    /// its first prediction of every tick (tick -> state)
    pub first: BTreeMap<u32, WalkState>,
    /// it has joined (the time, us)
    pub joined_us: Option<u64>,
    /// the virtual time it started
    pub start_us: u64,
    /// mismatches of the prediction counted at the last mark()
    pub mark_mismatches: u64,
    started: bool,
    rem: Vec<Remote>,
    feet: Vec<(u16, Vec3)>,
}

/// a session (see the module header)
pub struct Session {
    /// the settings
    pub cfg: SessionConfig,
    /// the network
    pub net: SimNet,
    /// the netcode's server
    pub server: NetServer<Arena>,
    /// the clients
    pub clients: Vec<Bc>,
    /// the map (every client predicts against it; the same as the server's)
    pub map: physics::World,
    /// the room of the session (the first client's)
    pub room: Option<RoomCode>,
    /// the server's state of every figure per tick: (id, tick) -> state
    pub server_states: BTreeMap<(u16, u32), WalkState>,
    /// the world snapshots of the last ticks (tick -> snapshot)
    pub worlds: BTreeMap<u32, Snapshot>,
    /// the server's tick times (us), when timed
    pub tick_us: Vec<u32>,
    /// the server's messages (connection, payload) folded by the checksum, when set
    pub record: Option<Vec<(u32, Vec<u8>)>>,
    /// the lag compensation's error of every shot: how far the target the server rewound (to the view time
    /// clamped to max_rewind, and unclamped) stood from where the shooter saw it (m)
    pub shot_errors: Vec<(f64, f64)>,
    /// shots waiting for the server's states of their view time: (target, tick, view, where it was seen)
    pending_shots: Vec<(u16, u32, u16, Vec3)>,
    clock: game::FixedTick,
    sev: Vec<ServerEvent>,
    cev: Vec<ClientEvent>,
}

impl Session {
    /// a session; the clients join from the first millisecond on, join_gap_ms apart
    pub fn new(cfg: SessionConfig) -> Session {
        let acfg = cfg.arena;
        let factory: game::net::server::RoomFactory<Arena> = alloc::boxed::Box::new(move |_| Arena::new(acfg));
        let server = NetServer::new(cfg.server, cfg.seed ^ 0x5e5e, factory);
        let net = SimNet::new(cfg.seed, cfg.chan);
        let map = arena::map(&cfg.arena);
        let mut clients = Vec::new();
        let t0 = net.now_us;
        for k in 0..cfg.players {
            let ws = cfg.ws_every > 0 && k % cfg.ws_every == cfg.ws_every - 1;
            let opts = ClientOptions { name: alloc::format!("Bot {}", k + 1), room: None, ..ClientOptions::default() };
            let client = NetClient::new(opts, Walker::default(), arena::schema(), arena::baseline(&cfg.arena));
            let bot = Bot::new(BotConfig { seed: cfg.seed.wrapping_mul(31).wrapping_add(k as u64 + 1), fire: cfg.fire, half: cfg.arena.half, rate: cfg.server.rate, ..BotConfig::default() });
            clients.push(Bc { peer: usize::MAX, client, bot, ws, first: BTreeMap::new(), joined_us: None, start_us: t0 + cfg.join_after_ms * 1000 + k as u64 * cfg.join_gap_ms * 1000, mark_mismatches: 0, started: false, rem: Vec::new(), feet: Vec::new() });
        }
        Session {
            clock: game::FixedTick::new(cfg.server.rate).with_max_steps(64),
            cfg,
            net,
            server,
            clients,
            map,
            room: None,
            server_states: BTreeMap::new(),
            worlds: BTreeMap::new(),
            tick_us: Vec::new(),
            record: None,
            shot_errors: Vec::new(),
            pending_shots: Vec::new(),
            sev: Vec::new(),
            cev: Vec::new(),
        }
    }

    fn client_cfg(&self) -> ClientConfig {
        ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() }
    }

    /// set every client's weather
    pub fn set_weather(&mut self, w: Weather) {
        for c in &self.clients {
            if c.peer != usize::MAX {
                self.net.set_weather(c.peer, w);
            }
        }
        self.cfg.weather = w;
    }

    /// remember every client's mismatch count now (checks count from here)
    pub fn mark(&mut self) {
        for c in self.clients.iter_mut() {
            c.mark_mismatches = c.client.predictor().stats().mismatches;
        }
    }

    /// a client more, joining the session's room now -> its index
    pub fn add_client(&mut self, ws: bool) -> usize {
        let k = self.clients.len();
        let opts = ClientOptions { name: alloc::format!("Bot {}", k + 1), room: self.room, ..ClientOptions::default() };
        let client = NetClient::new(opts, Walker::default(), arena::schema(), arena::baseline(&self.cfg.arena));
        let bot = Bot::new(BotConfig { seed: self.cfg.seed.wrapping_mul(31).wrapping_add(k as u64 + 1), fire: self.cfg.fire, half: self.cfg.arena.half, rate: self.cfg.server.rate, ..BotConfig::default() });
        self.clients.push(Bc { peer: usize::MAX, client, bot, ws, first: BTreeMap::new(), joined_us: None, start_us: self.net.now_us, mark_mismatches: 0, started: false, rem: Vec::new(), feet: Vec::new() });
        k
    }

    /// run `ms` milliseconds of virtual time
    pub fn run(&mut self, ms: u64) {
        for _ in 0..ms {
            self.step_ms();
        }
    }

    fn step_ms(&mut self) {
        self.net.step(1000);
        let now_us = self.net.now_us;
        let now_ns = now_us * 1000;
        // clients that start now
        let ccfg = self.client_cfg();
        for k in 0..self.clients.len() {
            if !self.clients[k].started && now_us >= self.clients[k].start_us {
                // the first client makes the room, the others join it once it exists
                if k > 0 || self.cfg.room.is_some() {
                    let room = self.cfg.room.or(self.room);
                    if room.is_none() {
                        continue;
                    }
                    let c = &mut self.clients[k];
                    let name = alloc::format!("Bot {}", k + 1);
                    let opts = ClientOptions { name, room, ..ClientOptions::default() };
                    c.client = NetClient::new(opts, Walker::default(), arena::schema(), arena::baseline(&self.cfg.arena));
                }
                let w = self.cfg.weather;
                let peer = if self.clients[k].ws { self.net.add_ws(ccfg, w) } else { self.net.add_udp(ccfg, w) };
                self.clients[k].peer = peer;
                self.clients[k].started = true;
            }
        }
        // the server
        self.net.server_events(&mut self.sev);
        for e in &self.sev {
            self.server.on_event(e);
        }
        let n = self.clock.advance_to(now_ns);
        for _ in 0..n {
            let t0 = if self.cfg.time_ticks { plat::time_us() } else { 0 };
            self.server.step();
            if self.cfg.time_ticks {
                self.tick_us.push((plat::time_us() - t0).min(u32::MAX as u64) as u32);
            }
            let t = self.server.tick().wrapping_sub(1);
            self.record_tick(t);
        }
        while let Some((id, b)) = self.server.take_out() {
            if let Some(r) = &mut self.record {
                r.push((id, b.clone()));
            }
            let _ = self.net.server_send(id, &b);
        }
        while let Some(id) = self.server.take_close() {
            self.net.server_disconnect(id);
        }
        // the clients
        for k in 0..self.clients.len() {
            if !self.clients[k].started {
                continue;
            }
            let peer = self.clients[k].peer;
            self.net.link_events(peer, Which::Main, &mut self.cev);
            for e in &self.cev {
                self.clients[k].client.on_event(&self.map, e, now_ns);
            }
            if self.clients[k].client.fetch_request().is_some() {
                let cfg = ClientConfig { max_payload: 64 * 1024, ..ccfg };
                self.net.open_fetch(peer, cfg);
            }
            self.net.link_events(peer, Which::Fetch, &mut self.cev);
            for e in &self.cev {
                self.clients[k].client.on_fetch_event(&self.map, e, now_ns);
            }
            let c = &mut self.clients[k];
            if c.joined_us.is_none() && c.client.phase() == Phase::Playing {
                c.joined_us = Some(now_us);
                if self.room.is_none() {
                    self.room = c.client.welcome().map(|w| w.room);
                }
            }
            let steps = c.client.advance(now_ns);
            for _ in 0..steps {
                c.client.remotes(&mut c.rem);
                remote_feet(&c.rem, &mut c.feet);
                let own = c.client.predicted().filter(|_| c.client.predictor().known()).map(|s| s.feet());
                let input = c.bot.think(own, &c.feet);
                let t = c.client.step(&self.map, input);
                if let Some(s) = c.client.predicted() {
                    c.first.insert(t, s);
                }
                // a shot: the target the bot aimed at (the nearest it saw) and where it saw it
                if input.buttons & game::net::walker::BTN_FIRE != 0 {
                    if let Some(me) = own {
                        let near = c.feet.iter().min_by(|a, b| {
                            let da = (a.1.x - me.x) * (a.1.x - me.x) + (a.1.z - me.z) * (a.1.z - me.z);
                            let db = (b.1.x - me.x) * (b.1.x - me.x) + (b.1.z - me.z) * (b.1.z - me.z);
                            da.partial_cmp(&db).unwrap_or(core::cmp::Ordering::Equal)
                        });
                        if let Some(&(id, seen)) = near {
                            // (the view the client put into the input it sent)
                            let view = match c.client.render_time() {
                                Some((a, f)) => game::net::quant::round_i64((tick_diff(t, a) as f64 - f) * 256.0).clamp(0, 65535) as u16,
                                None => 0,
                            };
                            self.pending_shots.push((id, t, view, seen));
                        }
                    }
                }
            }
            while c.first.len() > 1200 {
                c.first.pop_first();
            }
            while let Some(b) = self.clients[k].client.take_out() {
                let _ = self.net.link_send(peer, Which::Main, &b);
            }
            while let Some(b) = self.clients[k].client.take_fetch_out() {
                let _ = self.net.link_send(peer, Which::Fetch, &b);
            }
        }
    }

    fn record_tick(&mut self, t: u32) {
        let Some(code) = self.room else { return };
        let Some(a) = self.server.room(&code) else { return };
        for (id, f) in a.figures() {
            self.server_states.insert((*id, t), f.state);
        }
        // the shots of this tick: the target rewound as the server does (clamped) and without the limit
        let max_back = a.config().max_rewind;
        let states = &self.server_states;
        let at = |id: u16, view: u16, tick: u32, max: u32| -> Option<Vec3> {
            let (vt, frac) = game::net::lagcomp::view_time(tick, view, max);
            let p = states.get(&(id, vt))?.feet();
            let q = states.get(&(id, vt.wrapping_add(1))).map(|s| s.feet()).unwrap_or(p);
            Some(Vec3::new(p.x + (q.x - p.x) * frac, p.y + (q.y - p.y) * frac, p.z + (q.z - p.z) * frac))
        };
        let mut keep = Vec::new();
        for &(id, tick, view, seen) in &self.pending_shots {
            if tick_diff(tick, t) > 0 {
                keep.push((id, tick, view, seen));
                continue;
            }
            if let (Some(a), Some(b)) = (at(id, view, tick, max_back), at(id, view, tick, 255)) {
                self.shot_errors.push(((a - seen).length(), (b - seen).length()));
            }
        }
        self.pending_shots = keep;
        let mut w = Snapshot::new(t);
        a.snapshot(t, &mut w);
        self.worlds.insert(t, w);
        while self.worlds.len() > 64 {
            self.worlds.pop_first();
        }
        // forget states older than 1200 ticks
        if self.server_states.len() > 40_000 {
            let keep = t.wrapping_sub(1200);
            self.server_states.retain(|(_, tk), _| tick_diff(*tk, keep) >= 0);
        }
    }

    /// the clients' first predictions of ticks from `from_us` on (by the time the tick was predicted is not
    /// kept: by tick) compared with the server's states -> (compared, different)
    pub fn first_vs_server(&self, from_tick: Option<u32>) -> (u64, u64) {
        let (mut n, mut bad) = (0u64, 0u64);
        for c in &self.clients {
            let own = c.client.own_id();
            if own == 0 {
                continue;
            }
            for (t, s) in &c.first {
                if from_tick.is_some_and(|f| tick_diff(*t, f) < 0) {
                    continue;
                }
                if let Some(srv) = self.server_states.get(&(own, *t)) {
                    n += 1;
                    if srv != s {
                        bad += 1;
                    }
                }
            }
        }
        (n, bad)
    }

    /// clients in a room
    pub fn playing(&self) -> usize {
        self.clients.iter().filter(|c| c.client.phase() == Phase::Playing).count()
    }

    /// a short line of a client's state (for failure messages)
    pub fn describe(&self, k: usize) -> String {
        let c = &self.clients[k];
        let st = c.client.stats();
        let ps = c.client.predictor().stats();
        alloc::format!(
            "client {} ({}) {:?}: snapshots {} no_base {} bad {} stale {} mismatches {} resets {} margin {:?} speed {} jumps {}",
            k,
            if c.ws { "ws" } else { "udp" },
            c.client.phase(),
            st.snapshots,
            st.no_base,
            st.bad,
            st.stale,
            ps.mismatches,
            ps.resets,
            c.client.clock().margin(),
            c.client.clock().speed_ppm(),
            c.client.clock().jumps()
        )
    }
}

/// percentiles of a list of samples (sorted in place): (mean, p50, p99, max)
pub fn stats_of(v: &mut [f64]) -> (f64, f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    (mean, v[v.len() / 2], v[(v.len() * 99 / 100).min(v.len() - 1)], v[v.len() - 1])
}
