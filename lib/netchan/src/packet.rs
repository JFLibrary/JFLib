// packet -- the channel's wire format, version 2 (docs/TRANSPORT.md "Paketformat")
//
//   byte 0: VERSION << 4 | kind; then, little-endian:
//   CONNECT     client -> server  protocol u32, nonce u64              13 bytes (UDP: padded to 96)
//   CHALLENGE   server -> client  nonce u64, time u32, mac [16]        29 bytes (UDP only)
//   RESPONSE    client -> server  protocol u32, nonce u64, time u32,   66 bytes (UDP only, padded
//                                 mac [16], flags u8, key [32]                   to 96)
//   ACCEPT      server -> client  nonce u64, token u32, max payload    63 bytes
//                                 u16, key [32], confirm [16]
//   DENY        server -> client  nonce u64, reason u8                 10 bytes
//   DATA        both              token u32, seq u16, ack u16, ack bits u32, payload
//                                                                      13 + n (UDP: + 16)
//   DISCONNECT  both              token u32, seq u16, reason u8          8 bytes (UDP: + 16)
//
// Over UDP the handshake agrees on keys (RESPONSE: the client's ephemeral X25519 key; ACCEPT: the
// server's and `confirm`, the proof that the server derived the same keys; seal.rs), and every
// DATA and DISCONNECT after it is sealed with ChaCha20-Poly1305: the payload is encrypted, the
// header before it is authenticated with it, and the 16-byte tag follows (DATA's payload as
// parse() returns it is the ciphertext and the tag).  A UDP client pads CONNECT and RESPONSE to
// HANDSHAKE_SIZE, and every answer of the server before a connection is smaller: a forged sender
// address gets back less than was sent (no amplification).  Over a WebSocket the CONNECT is not
// padded, CHALLENGE / RESPONSE do not occur (TCP has proved the address), ACCEPT's key and
// confirm are zeros and nothing is sealed (wss:// is the encryption there).  parse() reads
// untrusted bytes: exact sizes, no panics, no allocation.

use alloc::vec::Vec;

/// the format's version (the high nibble of byte 0)
pub const VERSION: u8 = 2;
/// a client asks for a connection
pub const KIND_CONNECT: u8 = 1;
/// the server's stateless cookie for a UDP client
pub const KIND_CHALLENGE: u8 = 2;
/// the client returns the cookie with its key
pub const KIND_RESPONSE: u8 = 3;
/// the server took the client
pub const KIND_ACCEPT: u8 = 4;
/// the server refused the client
pub const KIND_DENY: u8 = 5;
/// a message (or an empty keep-alive) with sequence number and acks
pub const KIND_DATA: u8 = 6;
/// one side ends the connection
pub const KIND_DISCONNECT: u8 = 7;

/// the bytes of a DATA packet before its payload
pub const DATA_HEADER: usize = 13;
/// the bytes of a DISCONNECT packet before its tag
pub const DISCONNECT_HEADER: usize = 8;
/// the tag after a sealed DATA or DISCONNECT (UDP)
pub const TAG: usize = 16;
/// the size a UDP client pads CONNECT and RESPONSE to (at most the server takes)
pub const HANDSHAKE_SIZE: usize = 96;
/// the largest payload of a UDP DATA packet that may be configured (1400 + 13 + 16 + 8 + 40
/// bytes stay below the 1500 of Ethernet also over IPv6)
pub const MAX_UDP_PAYLOAD: usize = 1400;

/// RESPONSE flags: the client knows the server's static key (ClientConfig::server_key), and the
/// keys include a Diffie-Hellman with it
pub const FLAG_STATIC_KEY: u8 = 1;

/// DENY: the server has max_connections clients
pub const DENY_FULL: u8 = 1;
/// DENY: the client speaks another protocol id (another game or version)
pub const DENY_PROTOCOL: u8 = 2;
/// DENY: the client's address has max_per_ip connections
pub const DENY_ADDRESS: u8 = 3;
/// DENY: the client expects a static key, and the server has none
pub const DENY_KEY: u8 = 4;

/// DISCONNECT: an orderly end
pub const BYE_NORMAL: u8 = 0;
/// DISCONNECT: nothing came for timeout_ms
pub const BYE_TIMEOUT: u8 = 1;
/// DISCONNECT: the peer broke the rate limits
pub const BYE_RATE: u8 = 2;
/// DISCONNECT: the peer sent something malformed
pub const BYE_PROTOCOL: u8 = 3;
/// DISCONNECT: the same address connected again
pub const BYE_REPLACED: u8 = 4;
/// DISCONNECT: the server stops
pub const BYE_SHUTDOWN: u8 = 5;

/// one packet, borrowed from the bytes it was read from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packet<'a> {
    /// a client asks: the app's protocol id, the nonce of this attempt
    Connect { protocol: u32, nonce: u64 },
    /// the server's cookie for a UDP client: the time (seconds) and the MAC over address, nonce, time
    Challenge { nonce: u64, time: u32, mac: [u8; 16] },
    /// the client returns the cookie with FLAG_* and its ephemeral X25519 public key
    Response { protocol: u32, nonce: u64, time: u32, mac: [u8; 16], flags: u8, key: [u8; 32] },
    /// the server took the client: the token of its packets, the largest payload it takes, its
    /// ephemeral X25519 public key and the proof of the keys (over a WebSocket: zeros)
    Accept { nonce: u64, token: u32, max_payload: u16, key: [u8; 32], confirm: [u8; 16] },
    /// the server refused the client (DENY_*)
    Deny { nonce: u64, reason: u8 },
    /// a message (empty: a keep-alive) with its number, the newest number received and the bits
    /// of the 32 before; over UDP `payload` is the ciphertext and its tag
    Data { token: u32, seq: u16, ack: u16, bits: u32, payload: &'a [u8] },
    /// one side ends the connection (BYE_*) under the sequence number it took; over UDP `tag`
    /// holds the 16 bytes of the seal, over a WebSocket nothing
    Disconnect { token: u32, seq: u16, reason: u8, tag: &'a [u8] },
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[i..i + 8]);
    u64::from_le_bytes(a)
}

fn arr<const N: usize>(b: &[u8], i: usize) -> [u8; N] {
    let mut m = [0u8; N];
    m.copy_from_slice(&b[i..i + N]);
    m
}

/// read one packet: None for anything that is not exactly a packet of this version (handshake
/// packets between their size and HANDSHAKE_SIZE, the fixed-size ones exactly, DISCONNECT with
/// or without its tag)
pub fn parse(b: &[u8]) -> Option<Packet<'_>> {
    let first = *b.first()?;
    if first >> 4 != VERSION {
        return None;
    }
    let n = b.len();
    let sized = |min: usize| n >= min && n <= HANDSHAKE_SIZE;
    match first & 0x0f {
        KIND_CONNECT if sized(13) => Some(Packet::Connect { protocol: u32_at(b, 1), nonce: u64_at(b, 5) }),
        KIND_CHALLENGE if n == 29 => Some(Packet::Challenge { nonce: u64_at(b, 1), time: u32_at(b, 9), mac: arr(b, 13) }),
        KIND_RESPONSE if sized(66) => Some(Packet::Response {
            protocol: u32_at(b, 1),
            nonce: u64_at(b, 5),
            time: u32_at(b, 13),
            mac: arr(b, 17),
            flags: b[33],
            key: arr(b, 34),
        }),
        KIND_ACCEPT if n == 63 => Some(Packet::Accept { nonce: u64_at(b, 1), token: u32_at(b, 9), max_payload: u16_at(b, 13), key: arr(b, 15), confirm: arr(b, 47) }),
        KIND_DENY if n == 10 => Some(Packet::Deny { nonce: u64_at(b, 1), reason: b[9] }),
        KIND_DATA if n >= DATA_HEADER => {
            Some(Packet::Data { token: u32_at(b, 1), seq: u16_at(b, 5), ack: u16_at(b, 7), bits: u32_at(b, 9), payload: &b[DATA_HEADER..] })
        }
        KIND_DISCONNECT if n == DISCONNECT_HEADER || n == DISCONNECT_HEADER + TAG => {
            Some(Packet::Disconnect { token: u32_at(b, 1), seq: u16_at(b, 5), reason: b[7], tag: &b[DISCONNECT_HEADER..] })
        }
        _ => None,
    }
}

impl Packet<'_> {
    /// append the packet to `out`; `pad`: CONNECT and RESPONSE filled with zeros to HANDSHAKE_SIZE.
    /// DATA and DISCONNECT are written as given (Conn seals them)
    pub fn write(&self, out: &mut Vec<u8>, pad: bool) {
        let start = out.len();
        match *self {
            Packet::Connect { protocol, nonce } => {
                out.push(VERSION << 4 | KIND_CONNECT);
                out.extend_from_slice(&protocol.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
            }
            Packet::Challenge { nonce, time, mac } => {
                out.push(VERSION << 4 | KIND_CHALLENGE);
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&time.to_le_bytes());
                out.extend_from_slice(&mac);
            }
            Packet::Response { protocol, nonce, time, mac, flags, key } => {
                out.push(VERSION << 4 | KIND_RESPONSE);
                out.extend_from_slice(&protocol.to_le_bytes());
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&time.to_le_bytes());
                out.extend_from_slice(&mac);
                out.push(flags);
                out.extend_from_slice(&key);
            }
            Packet::Accept { nonce, token, max_payload, key, confirm } => {
                out.push(VERSION << 4 | KIND_ACCEPT);
                out.extend_from_slice(&nonce.to_le_bytes());
                out.extend_from_slice(&token.to_le_bytes());
                out.extend_from_slice(&max_payload.to_le_bytes());
                out.extend_from_slice(&key);
                out.extend_from_slice(&confirm);
            }
            Packet::Deny { nonce, reason } => {
                out.push(VERSION << 4 | KIND_DENY);
                out.extend_from_slice(&nonce.to_le_bytes());
                out.push(reason);
            }
            Packet::Data { token, seq, ack, bits, payload } => {
                out.push(VERSION << 4 | KIND_DATA);
                out.extend_from_slice(&token.to_le_bytes());
                out.extend_from_slice(&seq.to_le_bytes());
                out.extend_from_slice(&ack.to_le_bytes());
                out.extend_from_slice(&bits.to_le_bytes());
                out.extend_from_slice(payload);
            }
            Packet::Disconnect { token, seq, reason, tag } => {
                out.push(VERSION << 4 | KIND_DISCONNECT);
                out.extend_from_slice(&token.to_le_bytes());
                out.extend_from_slice(&seq.to_le_bytes());
                out.push(reason);
                out.extend_from_slice(tag);
            }
        }
        if pad && matches!(self, Packet::Connect { .. } | Packet::Response { .. }) {
            out.resize(start + HANDSHAKE_SIZE, 0);
        }
    }

    /// the packet as new bytes
    pub fn to_vec(&self, pad: bool) -> Vec<u8> {
        let mut v = Vec::with_capacity(match self {
            Packet::Data { payload, .. } => DATA_HEADER + payload.len() + TAG,
            _ => HANDSHAKE_SIZE,
        });
        self.write(&mut v, pad);
        v
    }
}

/// whether sequence number `a` is newer than `b` (wrapping: within half the range ahead)
pub fn seq_newer(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}
