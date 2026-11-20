// local -- a server and its clients in this process, on game::net's simulated network (netchan's cores on a virtual
// clock): the practice mode (one person against the server's bots, everywhere, also in the browser) and the tests
// (netsim, lagcomp, the checksum, the measurements)
//
//   Local::new          a NetServer with a Match per room on the simulated network
//   add_client          a NetClient<Mover> over UDP or WebSocket in some weather; the first makes the room, the
//                       others join it once its code is known
//   step_ms(input)      one millisecond: the network delivers, the server takes its events and runs its ticks, every
//                       client takes its events, runs its ticks (input(k, client) makes each tick's input) and sends
//   tick_log            when set, the server's movement state of every figure per tick (the tests' reference)

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use game::net::client::{ClientOptions, NetClient, Phase};
use game::net::input::NetInput;
use game::net::server::{NetServer, RoomFactory, ServerOptions};
use game::net::sim::{SimNet, Weather, Which};
use game::net::RoomCode;
use game1_core::map::Map;
use game1_core::movement::{MoveState, Mover};
use game1_core::rules::{schema, PROTOCOL, RATE};
use game1_core::sim::{Match, MatchConfig};
use netchan::endpoint::{ClientConfig, ClientEvent, ServerConfig, ServerEvent};

/// the network of the practice mode: a millisecond each way, nothing lost
pub const LOCAL: Weather = Weather { loss: 0.0, dup: 0.0, reorder: 0.0, extra_us: 0, delay_us: 1000, jitter_us: 0 };

/// one client of a Local
pub struct LocalClient {
    pub client: NetClient<Mover>,
    pub peer: usize,
    pub ws: bool,
    pub name: String,
    started: bool,
    start_us: u64,
    weather: Weather,
}

/// a server and clients on the simulated network (see the module header)
pub struct Local {
    pub net: SimNet,
    pub server: NetServer<Match>,
    pub clients: Vec<LocalClient>,
    pub map: Arc<Map>,
    /// the room of the first client
    pub room: Option<RoomCode>,
    /// (id, tick) -> the server's state after that tick, when Some
    pub tick_log: Option<BTreeMap<(u16, u32), MoveState>>,
    /// when Some, the time of every server tick (us; the measurement)
    pub tick_us: Option<Vec<u32>>,
    clock: game::FixedTick,
    sev: Vec<ServerEvent>,
    cev: Vec<ClientEvent>,
}

impl Local {
    /// a server with these options and rooms on `map`
    pub fn new(seed: u64, opts: ServerOptions, room: MatchConfig, map: Arc<Map>) -> Local {
        let m2 = map.clone();
        let mut n = 0u64;
        let factory: RoomFactory<Match> = Box::new(move |_| {
            n += 1;
            Match::new(MatchConfig { seed: room.seed.wrapping_add(n), ..room }, m2.clone())
        });
        let chan = ServerConfig { protocol: PROTOCOL, packets_per_s: 100_000, bytes_per_s: 1 << 28, ..ServerConfig::default() };
        Local {
            net: SimNet::new(seed, chan),
            server: NetServer::new(ServerOptions { rate: RATE, ..opts }, seed ^ 0x5e5e, factory),
            clients: Vec::new(),
            map,
            room: None,
            tick_log: None,
            tick_us: None,
            clock: game::FixedTick::new(RATE).with_max_steps(64),
            sev: Vec::new(),
            cev: Vec::new(),
        }
    }

    fn cfg() -> ClientConfig {
        ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() }
    }

    /// a client more (it starts after `after_ms`) -> its index
    pub fn add_client(&mut self, name: &str, ws: bool, weather: Weather, after_ms: u64) -> usize {
        let opts = ClientOptions { name: String::from(name), room: None, ..ClientOptions::default() };
        let client = NetClient::new(opts, Mover::default(), schema(), Match::empty_baseline());
        self.clients.push(LocalClient { client, peer: usize::MAX, ws, name: String::from(name), started: false, start_us: self.net.now_us + after_ms * 1000, weather });
        self.clients.len() - 1
    }

    /// the network of one client from now on
    pub fn set_weather(&mut self, k: usize, w: Weather) {
        self.clients[k].weather = w;
        if self.clients[k].peer != usize::MAX {
            self.net.set_weather(self.clients[k].peer, w);
        }
    }

    /// one millisecond (see the module header) -> the clients that stepped a tick in it (index, tick)
    pub fn step_ms(&mut self, input: &mut dyn FnMut(usize, &NetClient<Mover>) -> NetInput) -> Vec<(usize, u32)> {
        self.net.step(1000);
        let now_us = self.net.now_us;
        let now_ns = now_us * 1000;
        let ccfg = Local::cfg();
        for k in 0..self.clients.len() {
            let c = &self.clients[k];
            if c.started || now_us < c.start_us {
                continue;
            }
            if k > 0 && self.room.is_none() {
                continue;
            }
            let name = c.name.clone();
            let opts = ClientOptions { name, room: if k > 0 { self.room } else { None }, ..ClientOptions::default() };
            let client = NetClient::new(opts, Mover::default(), schema(), Match::empty_baseline());
            let w = c.weather;
            let peer = if c.ws { self.net.add_ws(ccfg, w) } else { self.net.add_udp(ccfg, w) };
            let c = &mut self.clients[k];
            c.client = client;
            c.peer = peer;
            c.started = true;
        }
        // the server
        self.net.server_events(&mut self.sev);
        for e in &self.sev {
            self.server.on_event(e);
        }
        let n = self.clock.advance_to(now_ns);
        for _ in 0..n {
            let t0 = if self.tick_us.is_some() { plat::time_us() } else { 0 };
            self.server.step();
            if let Some(v) = &mut self.tick_us {
                v.push((plat::time_us() - t0).min(u32::MAX as u64) as u32);
            }
            if self.tick_log.is_some() {
                let t = self.server.tick().wrapping_sub(1);
                if let Some(m) = self.room.and_then(|r| self.server.room(&r)) {
                    let log = self.tick_log.as_mut().map(|l| (l, m.figures()));
                    if let Some((log, figs)) = log {
                        for (id, f) in figs {
                            log.insert((*id, t), f.state);
                        }
                        if log.len() > 200_000 {
                            let keep = t.wrapping_sub(3000);
                            log.retain(|(_, tk), _| (tk.wrapping_sub(keep) as i32) >= 0);
                        }
                    }
                }
            }
        }
        while let Some((id, b)) = self.server.take_out() {
            let _ = self.net.server_send(id, &b);
        }
        while let Some(id) = self.server.take_close() {
            self.net.server_disconnect(id);
        }
        // the clients
        let mut stepped = Vec::new();
        let map = self.map.clone();
        for k in 0..self.clients.len() {
            if !self.clients[k].started {
                continue;
            }
            let peer = self.clients[k].peer;
            self.net.link_events(peer, Which::Main, &mut self.cev);
            for e in &self.cev {
                self.clients[k].client.on_event(&map.world, e, now_ns);
            }
            if self.room.is_none() && self.clients[k].client.phase() == Phase::Playing {
                self.room = self.clients[k].client.welcome().map(|w| w.room);
            }
            let c = &mut self.clients[k];
            for _ in 0..c.client.advance(now_ns) {
                let i = input(k, &c.client);
                let t = c.client.step(&map.world, i);
                stepped.push((k, t));
            }
            while let Some(b) = self.clients[k].client.take_out() {
                let _ = self.net.link_send(peer, Which::Main, &b);
            }
        }
        stepped
    }

    /// the clients in a room
    pub fn playing(&self) -> usize {
        self.clients.iter().filter(|c| c.client.phase() == Phase::Playing).count()
    }

    /// the room's match on the server
    pub fn room_match(&self) -> Option<&Match> {
        self.room.and_then(|r| self.server.room(&r))
    }

    /// the room's match on the server, to change
    pub fn room_match_mut(&mut self) -> Option<&mut Match> {
        let r = self.room?;
        self.server.room_mut(&r)
    }

    /// a short line of a client (failure messages)
    pub fn describe(&self, k: usize) -> String {
        let c = &self.clients[k];
        let st = c.client.stats();
        let ps = c.client.predictor().stats();
        format!(
            "client {} ({}) {:?}: snapshots {} no_base {} bad {} mismatches {} resets {} margin {:?}",
            k,
            if c.ws { "ws" } else { "udp" },
            c.client.phase(),
            st.snapshots,
            st.no_base,
            st.bad,
            ps.mismatches,
            ps.resets,
            c.client.clock().margin()
        )
    }
}
