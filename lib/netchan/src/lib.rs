// netchan -- message channels for games (G7d, docs/TRANSPORT.md)
//
//   Client::connect(Endpoint::Udp(addr) | Endpoint::WebSocket(url), ClientConfig)
//            poll() -> ClientEvent (Connected, Message { seq, data }, Delivered(seq), Lost(seq),
//            Disconnected(reason)), send(payload) -> seq, state, reliable, max_payload, stats
//   Server::new(ServerConfig) [.bind_udp(addr)] [.listen_ws(addr, path) | .ws_endpoint()]
//            poll() -> ServerEvent, send(id, payload) -> seq, disconnect(id), stats(id)
//            (Windows, Linux, Apple: it needs netio's HTTP server)
//   WebSocket::connect(url): one non-blocking client type for the browser and the desktop
//   packet / conn / endpoint: the wire format, the sequence numbers and acks, the handshake --
//            without I/O, for tests on a simulated network and for other transports
//
// The same channel over two transports: unreliable and unordered over UDP (a handshake with a
// stateless cookie, a token in every packet, keep-alives and timeouts), reliable and ordered
// over a WebSocket (the browser's only way, docs/GAME.md E8).  Both carry the same header: a
// sequence number per packet and acks of the newest 33 received, so the netcode (G7e) learns
// which packets arrived over either.  Limits everywhere: payload sizes, packets and bytes a
// second per connection, connections per server and per address, handshake answers a second.
//
// Why a crate of its own and not netio: netio is not built for the browser and Android (plat had
// no sockets there), and the client side must run in the browser; the WebSocket client type
// needs netio on the desktop (names on Linux go through netio::dns) and plat's JS host in the
// browser, which only a crate above both can choose per target.  Not lib/game either (G7b builds
// it at the same time; the transport has no game in it).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;

pub mod client;
pub mod conn;
pub mod endpoint;
pub mod packet;
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
mod reap;
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod server;
pub mod ws;

pub use client::{Client, Endpoint};
pub use conn::Stats;
pub use endpoint::{ClientConfig, ClientEvent, ServerConfig, ServerEvent, State};
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub use server::{Server, WsEndpoint};
pub use ws::{WebSocket, WsMessage, WsOptions, WsState};

/// which transport a connection runs over
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// UDP: unreliable, unordered
    Udp,
    /// a WebSocket: reliable, ordered
    WebSocket,
}

/// why a connection ended
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// this side ended it
    Local,
    /// the peer sent DISCONNECT with this code (packet::BYE_*)
    Peer(u8),
    /// nothing came for timeout_ms
    Timeout,
    /// the handshake did not finish within connect_ms
    ConnectTimeout,
    /// the server refused: it is full (or the address has max_per_ip connections)
    Full,
    /// the server speaks another protocol id
    Protocol,
    /// the peer broke the rate limits (the server ended it)
    RateLimit,
    /// the WebSocket under it closed with this code (1006: without a close frame)
    Closed(u16),
    /// the same address made a new handshake
    Replaced,
}

/// why a call failed
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// not on this target: UDP in the browser; everything on Android for now (plat has no sockets
    /// there yet, docs/ANDROID.md)
    Unsupported,
    /// the network; NetErrorKind::Denied: the app's permissions (net.connect / net.listen)
    Net(plat::net::NetError),
    /// not a ws:// or wss:// URL (or "/path" in the browser)
    Url(String),
    /// an empty payload, or one over the limit of the connection
    Size,
    /// more waits to be sent than the WebSocket's queue takes (WsOptions::max_unsent)
    Backlog,
    /// not connected (yet, or any more)
    NotConnected,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Unsupported => f.write_str("not supported on this target"),
            Error::Net(e) => write!(f, "{}", e),
            Error::Url(u) => write!(f, "not a WebSocket URL: {}", u),
            Error::Size => f.write_str("payload empty or too large"),
            Error::Backlog => f.write_str("too much waits to be sent"),
            Error::NotConnected => f.write_str("not connected"),
        }
    }
}

impl From<plat::net::NetError> for Error {
    fn from(e: plat::net::NetError) -> Self {
        Error::Net(e)
    }
}
