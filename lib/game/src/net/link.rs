// link -- the netcode on real sockets: netchan's Server and Client under NetServer and NetClient
//
//   serve(server, game, now_ns)   one round of a server's loop: netchan's events into the game, the ticks
//                                 due, the game's messages out, the connections it ends closed (Windows,
//                                 Linux, Apple: netchan::Server needs netio)
//   NetLink<M>                    a client's channel and, when WELCOME asks for it, the fetch's WebSocket
//                                 (ws://host:port/path of the server's UDP address); pump(map, now) reads
//                                 both, flush() sends what the game queued, leave() ends it cleanly (LEAVE,
//                                 then netchan's DISCONNECT: the server removes the figure at once)
//
// A server's loop: serve(..) then server.wait(ms to the next tick) (never plat::sleep_ms(1) on Windows:
// docs/TRANSPORT.md).  A client's frame: link.pump(map, now), n = link.game.advance(now), n times
// link.game.step(map, input), link.flush().

use alloc::format;
use alloc::string::String;

use netchan::{Client, ClientConfig, Endpoint};

use super::client::NetClient;
use super::predict::Movement;

/// one round of a server's loop (see the module header) -> the ticks run
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub fn serve<S: super::server::RoomSim>(server: &mut netchan::Server, game: &mut super::server::NetServer<S>, now_ns: u64) -> u32 {
    while let Some(e) = server.poll() {
        game.on_event(&e);
    }
    let n = game.advance(now_ns);
    while let Some((id, b)) = game.take_out() {
        let _ = server.send(id, &b);
    }
    while let Some(id) = game.take_close() {
        server.disconnect(id);
    }
    n
}

/// a client's channels (see the module header)
pub struct NetLink<M: Movement> {
    /// the game's client
    pub game: NetClient<M>,
    chan: Client,
    fetch: Option<Client>,
    cfg: ClientConfig,
    host: Option<String>,
    path: String,
    /// sends netchan refused (Size, Backlog, NotConnected)
    pub send_errors: u64,
}

impl<M: Movement> NetLink<M> {
    /// connect to `ep`; `path` is the server's WebSocket path (for a fetch of a UDP client)
    pub fn connect(ep: Endpoint, cfg: ClientConfig, game: NetClient<M>, path: &str) -> Result<NetLink<M>, netchan::Error> {
        let mut game = game;
        game.set_reliable(matches!(ep, Endpoint::WebSocket(_)));
        let host = match &ep {
            Endpoint::Udp(a) => Some(match a.ip {
                plat::net::IpAddr::V4(b) => format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]),
                plat::net::IpAddr::V6(_) => format!("[{}]", a.ip),
            }),
            Endpoint::WebSocket(_) => None,
        };
        let chan = Client::connect(ep, cfg)?;
        Ok(NetLink { game, chan, fetch: None, cfg, host, path: String::from(path), send_errors: 0 })
    }

    /// read the channels, feed the game, open the fetch when asked
    pub fn pump(&mut self, map: &physics::World, now_ns: u64) {
        while let Some(e) = self.chan.poll() {
            self.game.on_event(map, &e, now_ns);
        }
        if let Some((port, _)) = self.game.fetch_request() {
            if let Some(h) = &self.host {
                let url = format!("ws://{}:{}{}", h, port, self.path);
                let cfg = ClientConfig { max_payload: 64 * 1024, ..self.cfg };
                self.fetch = Client::connect(Endpoint::WebSocket(url), cfg).ok();
            }
        }
        let mut done = false;
        if let Some(f) = &mut self.fetch {
            while let Some(e) = f.poll() {
                done |= matches!(e, netchan::ClientEvent::Disconnected(_));
                self.game.on_fetch_event(map, &e, now_ns);
            }
        }
        if done {
            self.fetch = None;
        }
        self.flush();
    }

    /// send what the game queued
    pub fn flush(&mut self) {
        while let Some(b) = self.game.take_out() {
            if self.chan.send(&b).is_err() {
                self.send_errors += 1;
            }
        }
        while let Some(b) = self.game.take_fetch_out() {
            match &mut self.fetch {
                Some(f) => {
                    if f.send(&b).is_err() {
                        self.send_errors += 1;
                    }
                }
                None => self.send_errors += 1,
            }
        }
    }

    /// leave the room: LEAVE (the server removes the figure at once), then netchan's DISCONNECT (over UDP three
    /// times) and the end of a fetch's WebSocket.  Dropping a NetLink only does the last two
    pub fn leave(&mut self) {
        self.game.leave();
        self.flush();
        self.chan.disconnect();
        if let Some(f) = &mut self.fetch {
            f.disconnect();
        }
        self.fetch = None;
    }

    /// the main channel (its state, stats, round-trip time)
    pub fn channel(&self) -> &Client {
        &self.chan
    }

    /// a fetch's WebSocket is open
    pub fn fetching(&self) -> bool {
        self.fetch.is_some()
    }
}
