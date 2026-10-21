// Nonblocking per-instance local agent sessions. Framing and RPC semantics stay shared.

use crate::{Framer, Record};
use agent_core::{Dispatcher, Host, Protocol, RpcSession};
use alloc::{string::String, vec::Vec};
use plat::control_ipc::{Listener, Stream};

const MAX_CLIENTS: usize = 8;
const MAX_QUEUED: usize = 32;
const MAX_OUTPUT: usize = 2_097_152;

struct Peer {
    stream: Stream,
    framer: Framer,
    session: RpcSession,
    started: bool,
    queue: Vec<Record>,
    output: Vec<u8>,
    written: usize,
    closed: bool,
}

/// An explicitly created per-application local endpoint. No listener exists by default.
pub struct Server {
    listener: Option<Listener>,
    peers: Vec<Peer>,
    app: String,
    version: String,
    protocol: Protocol,
}

impl Server {
    /// Bind a current-user endpoint. A bounded ASCII slug is derived from the display name;
    /// discovery and protocol metadata retain the original Unicode app name.
    pub fn bind(app: &str, version: &str, protocol: Protocol) -> Result<Self, i32> {
        if app.is_empty() || app.len() > 4096 {
            return Err(22);
        }
        let suffix = match protocol {
            Protocol::Json => "-json",
            Protocol::Mcp => "-mcp",
        };
        let mut name = String::new();
        for character in app.chars() {
            if name.len() == 27 {
                break;
            }
            if character.is_ascii_alphanumeric() {
                name.push(character.to_ascii_lowercase());
            } else if !name.is_empty() && !name.ends_with('-') {
                name.push('-');
            }
        }
        while name.ends_with('-') {
            name.pop();
        }
        if name.is_empty() {
            name.push_str("app");
        }
        name.push_str(suffix);
        Ok(Self {
            listener: Some(Listener::bind(&name)?),
            peers: Vec::new(),
            app: app.into(),
            version: version.into(),
            protocol,
        })
    }
    /// Native endpoint address, empty after disabling/closing.
    pub fn endpoint(&self) -> &str {
        self.listener.as_ref().map(Listener::endpoint).unwrap_or("")
    }
    /// Number of accepted active connections, for a visible app status indicator.
    pub fn connected(&self) -> usize {
        self.peers.len()
    }
    /// Close the endpoint and all connections immediately.
    pub fn close(&mut self) {
        self.peers.clear();
        self.listener = None;
    }
    /// Process a bounded amount of available work without blocking the native UI loop.
    /// Returns true when at least one request was processed. A disabled dispatcher closes the endpoint.
    pub fn poll(&mut self, dispatcher: &Dispatcher, host: &mut impl Host) -> bool {
        if !dispatcher.enabled() {
            self.close();
            return false;
        }
        let Some(listener) = &mut self.listener else {
            return false;
        };
        if self.peers.len() < MAX_CLIENTS {
            if let Ok(Some(stream)) = listener.accept() {
                self.peers.push(Peer {
                    stream,
                    framer: Framer::default(),
                    session: RpcSession::new(self.protocol, &self.app, &self.version),
                    started: false,
                    queue: Vec::new(),
                    output: Vec::new(),
                    written: 0,
                    closed: false,
                });
            }
        }
        let mut processed = false;
        for peer in &mut self.peers {
            if !peer.output.is_empty() {
                match peer.stream.write(&peer.output[peer.written..]) {
                    Ok(Some(0)) | Err(_) => peer.closed = true,
                    Ok(Some(count)) => {
                        peer.written += count;
                        if peer.written == peer.output.len() {
                            peer.output.clear();
                            peer.written = 0;
                        }
                    }
                    Ok(None) => (),
                }
                // Read no further commands from a slow client until its response has drained.
                if peer.closed || !peer.output.is_empty() {
                    continue;
                }
            }
            let mut buffer = [0u8; 4096];
            if peer.queue.is_empty() {
                match peer.stream.read(&mut buffer) {
                    Ok(Some(0)) | Err(_) => peer.closed = true,
                    Ok(Some(count)) => {
                        peer.framer.feed(&buffer[..count], &mut |record| {
                            if peer.queue.len() < MAX_QUEUED {
                                peer.queue.push(record);
                            } else {
                                peer.closed = true;
                            }
                        });
                    }
                    Ok(None) => (),
                }
            }
            if peer.closed {
                continue;
            }
            let count = peer.queue.len().min(4);
            for record in peer.queue.drain(..count) {
                let response = match record {
                    Record::Line(line) if line.is_empty() => None,
                    Record::Line(line) => {
                        if !peer.started {
                            // The first explicit MCP version envelope selects that protocol for this
                            // connection; ordinary control requests keep JSON v1. Never switch later.
                            if let Some(protocol) = agent_core::rpc::requested_protocol(&line) {
                                if self.protocol == Protocol::Json && protocol == Protocol::Mcp {
                                    peer.session = RpcSession::new(Protocol::Mcp, &self.app, &self.version);
                                }
                                peer.started = true;
                            }
                        }
                        processed = true; peer.session.handle(dispatcher, host, &line)
                    }
                    Record::TooLong => Some(String::from("{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32600,\"message\":\"Request exceeds 1048576 bytes\"}}\n")),
                };
                if let Some(response) = response {
                    if response.len() > MAX_OUTPUT.saturating_sub(peer.output.len()) {
                        peer.closed = true;
                        break;
                    }
                    peer.output.extend_from_slice(response.as_bytes());
                }
            }
        }
        self.peers.retain(|peer| !peer.closed);
        processed
    }
}
