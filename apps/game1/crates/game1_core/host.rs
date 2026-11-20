// host -- game 1's server on real sockets (Windows, Linux, Apple): a netchan::Server (UDP and the WebSocket path,
// which the caller binds: game1-server, the client's loopback test) under game::net's NetServer with a Match per
// room.  The loop is one thread; the rooms of a tick run side by side on plat's worker pool (one room per core,
// NetServer::set_parallel: the same messages as on one thread).  Room codes and tickets come from the system's
// cryptographic random numbers (NetServer::secure)
//
//   Host::new(chan, ws_port, opts)   the server's state; rooms make their Match from the shared map
//   Host::round(now_ns)              one round of the loop: netchan's events in, the ticks due, the messages out
//   Host::wait_ms()                  how long the loop may sleep (chan.wait: never plat::sleep_ms on Windows)

use alloc::boxed::Box;
use alloc::sync::Arc;

use game::net::link::serve;
use game::net::server::{NetServer, ServerOptions};

use crate::map::Map;
use crate::rules::RATE;
use crate::sim::{Match, MatchConfig};

/// a host's settings
#[derive(Clone, Copy, Debug)]
pub struct HostOptions {
    /// the rooms' settings (bots, seed, ...)
    pub room: MatchConfig,
    /// the netcode's
    pub server: ServerOptions,
    /// rooms side by side on the worker pool
    pub parallel: bool,
}

impl Default for HostOptions {
    fn default() -> Self {
        HostOptions { room: MatchConfig::default(), server: ServerOptions { max_players: crate::rules::MAX_FIGURES, ..ServerOptions::default() }, parallel: true }
    }
}

/// a server on real sockets (see the module header)
pub struct Host {
    pub chan: netchan::Server,
    pub game: NetServer<Match>,
}

impl Host {
    /// a host on a bound netchan server (ws_port: its WebSocket's port, 0: none)
    pub fn new(chan: netchan::Server, ws_port: u16, opts: HostOptions, map: Arc<Map>) -> Host {
        let room = opts.room;
        let mut n = 0u64;
        let factory: game::net::server::RoomFactory<Match> = Box::new(move |_code| {
            n += 1;
            Match::new(MatchConfig { seed: room.seed.wrapping_add(n), ..room }, map.clone())
        });
        let server = ServerOptions { ws_port, rate: RATE, max_rewind_ms: room.max_rewind * 1000 / RATE, ..opts.server };
        let mut game = NetServer::secure(server, factory);
        game.set_parallel(opts.parallel);
        Host { chan, game }
    }

    /// one round of the loop -> the ticks run
    pub fn round(&mut self, now_ns: u64) -> u32 {
        serve(&mut self.chan, &mut self.game, now_ns)
    }

    /// sleep until a packet comes, at most `ms`
    pub fn wait(&mut self, ms: u32) {
        self.chan.wait(ms);
    }
}
