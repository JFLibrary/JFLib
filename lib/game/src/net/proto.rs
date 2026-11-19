// proto -- the netcode's messages over a netchan channel (one message a packet; byte 0 is the type, the
// rest a bit stream of bits.rs)
//
//   HELLO      client -> server   the version, a room code (none: make a new room), the player's name
//   WELCOME    server -> client   the room's code, the player's net id, rate and snapshot interval, the
//                                 server's tick now, the input margin to aim for, the lag compensation's
//                                 limit, and for a UDP client whose full state does not fit a datagram a
//                                 ticket and the WebSocket port to fetch it with
//   REFUSE     server -> client   why not (version, room full, no such room, no name, server full, ticket)
//   INPUT      client -> server   the newest snapshot it decoded, the full state it fetched (until
//                                 confirmed), its inputs (input.rs: the newest and those the server may lack)
//   SNAPSHOT   server -> client   the tick, the base (none, the map's baseline, or a tick back), the
//                                 player's id, up to which tick its inputs are complete, the margin of its
//                                 inputs, then the entities (snapshot.rs)
//   FETCH      client -> server   (on a WebSocket of its own) the ticket of WELCOME
//   FULL       server -> client   (on that WebSocket) a SNAPSHOT against the baseline without a budget
//
// Every reader returns None for anything malformed or too large (bytes from the network never panic).

use alloc::string::String;
use alloc::vec::Vec;

use super::bits::{BitReader, BitWriter};
use super::input::{read_inputs, write_inputs, NetInput};
use super::room::RoomCode;

/// the netcode's own version (HELLO); a game's version goes into netchan's protocol id
pub const VERSION: u16 = 1;

/// message types (byte 0)
pub const MSG_HELLO: u8 = 1;
/// see the module header
pub const MSG_WELCOME: u8 = 2;
/// see the module header
pub const MSG_REFUSE: u8 = 3;
/// see the module header
pub const MSG_INPUT: u8 = 4;
/// see the module header
pub const MSG_SNAPSHOT: u8 = 5;
/// see the module header
pub const MSG_FETCH: u8 = 6;
/// see the module header
pub const MSG_FULL: u8 = 7;

/// REFUSE: another version of the netcode
pub const REFUSE_VERSION: u8 = 1;
/// REFUSE: the room has its players
pub const REFUSE_ROOM_FULL: u8 = 2;
/// REFUSE: no room has that code
pub const REFUSE_NO_ROOM: u8 = 3;
/// REFUSE: the name is empty after cleaning
pub const REFUSE_NAME: u8 = 4;
/// REFUSE: the server has its rooms
pub const REFUSE_SERVER_FULL: u8 = 5;
/// REFUSE: the ticket of a FETCH is unknown or used
pub const REFUSE_TICKET: u8 = 6;
/// REFUSE: the message was not what the server expected
pub const REFUSE_PROTOCOL: u8 = 7;

/// HELLO
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// VERSION of the client
    pub version: u16,
    /// the room to join (None: make one)
    pub room: Option<RoomCode>,
    /// the player's name as typed (the server cleans it)
    pub name: String,
}

/// WELCOME
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// the room's code
    pub room: RoomCode,
    /// the net id of the player's figure
    pub player: u16,
    /// the server's ticks a second
    pub rate: u16,
    /// a snapshot every this many ticks
    pub snapshot_every: u8,
    /// the server's tick when it sent this
    pub tick: u32,
    /// the margin of inputs to aim for (ticks)
    pub margin: u8,
    /// how far back the server rewinds for hits (ticks)
    pub max_rewind: u8,
    /// (UDP) the full state does not fit a datagram: fetch it over a WebSocket with `ticket`
    pub fetch: bool,
    /// the ticket for FETCH
    pub ticket: u64,
    /// the port of the server's WebSocket (0: none)
    pub ws_port: u16,
    /// the bytes of the full state when the player joined
    pub full_bytes: u32,
}

/// a snapshot's base
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// nothing: every entity is an add
    None,
    /// the map's baseline (the client has it from the map)
    Map,
    /// the snapshot this many ticks back (1 ..= 255)
    Back(u8),
}

/// a SNAPSHOT's or FULL's header
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapHeader {
    /// the tick
    pub tick: u32,
    /// what the entities are encoded against
    pub base: Base,
    /// the net id of the receiver's figure (0: none)
    pub player: u16,
    /// the receiver's inputs are complete up to this tick
    pub have: u32,
    /// the smallest margin of its inputs since the last snapshot (None: none came)
    pub margin: Option<i8>,
}

/// an INPUT
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputMsg {
    /// the tick of the newest input
    pub newest: u32,
    /// the inputs, the newest first
    pub inputs: Vec<NetInput>,
    /// the newest snapshot the client decoded
    pub ack: Option<u32>,
    /// the FULL it fetched (repeated until the server bases on it)
    pub full: Option<u32>,
}

/// the type of a message (None: empty)
pub fn msg_type(b: &[u8]) -> Option<u8> {
    b.first().copied()
}

/// HELLO as bytes
pub fn write_hello(h: &Hello) -> Vec<u8> {
    let mut w = BitWriter::with_prefix(&[MSG_HELLO]);
    w.u16(h.version);
    w.bool(h.room.is_some());
    if let Some(r) = h.room {
        for c in r.0 {
            w.u8(c);
        }
    }
    let name = h.name.as_bytes();
    let n = name.len().min(64);
    w.u8(n as u8);
    for &c in &name[..n] {
        w.u8(c);
    }
    w.finish()
}

/// HELLO from bytes
pub fn read_hello(b: &[u8]) -> Option<Hello> {
    if msg_type(b)? != MSG_HELLO {
        return None;
    }
    let mut r = BitReader::new(&b[1..]);
    let version = r.u16();
    let room = if r.bool() {
        let mut c = [0u8; 6];
        for x in c.iter_mut() {
            *x = r.u8();
        }
        Some(RoomCode(c))
    } else {
        None
    };
    let n = r.u8() as usize;
    if n > 64 {
        return None;
    }
    let mut name = Vec::with_capacity(n);
    for _ in 0..n {
        name.push(r.u8());
    }
    if !r.ok() {
        return None;
    }
    // (a name cut inside a character loses that character)
    let name = match String::from_utf8(name) {
        Ok(s) => s,
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            let mut v = e.into_bytes();
            v.truncate(valid);
            String::from_utf8(v).unwrap_or_default()
        }
    };
    Some(Hello { version, room, name })
}

/// WELCOME as bytes
pub fn write_welcome(m: &Welcome) -> Vec<u8> {
    let mut w = BitWriter::with_prefix(&[MSG_WELCOME]);
    for c in m.room.0 {
        w.u8(c);
    }
    w.u16(m.player);
    w.u16(m.rate);
    w.u8(m.snapshot_every);
    w.u32(m.tick);
    w.u8(m.margin);
    w.u8(m.max_rewind);
    w.bool(m.fetch);
    w.u32(m.ticket as u32);
    w.u32((m.ticket >> 32) as u32);
    w.u16(m.ws_port);
    w.u32(m.full_bytes);
    w.finish()
}

/// WELCOME from bytes
pub fn read_welcome(b: &[u8]) -> Option<Welcome> {
    if msg_type(b)? != MSG_WELCOME {
        return None;
    }
    let mut r = BitReader::new(&b[1..]);
    let mut c = [0u8; 6];
    for x in c.iter_mut() {
        *x = r.u8();
    }
    let m = Welcome {
        room: RoomCode(c),
        player: r.u16(),
        rate: r.u16(),
        snapshot_every: r.u8(),
        tick: r.u32(),
        margin: r.u8(),
        max_rewind: r.u8(),
        fetch: r.bool(),
        ticket: r.u32() as u64 | (r.u32() as u64) << 32,
        ws_port: r.u16(),
        full_bytes: r.u32(),
    };
    (r.ok() && m.room.valid() && m.rate > 0 && m.snapshot_every > 0).then_some(m)
}

/// REFUSE as bytes
pub fn write_refuse(reason: u8) -> Vec<u8> {
    alloc::vec![MSG_REFUSE, reason]
}

/// REFUSE's reason
pub fn read_refuse(b: &[u8]) -> Option<u8> {
    (b.len() == 2 && b[0] == MSG_REFUSE).then(|| b[1])
}

/// INPUT as bytes
pub fn write_input(m: &InputMsg) -> Vec<u8> {
    let mut w = BitWriter::with_prefix(&[MSG_INPUT]);
    w.bool(m.ack.is_some());
    if let Some(a) = m.ack {
        w.u32(a);
    }
    w.bool(m.full.is_some());
    if let Some(f) = m.full {
        w.u32(f);
    }
    write_inputs(&mut w, m.newest, &m.inputs);
    w.finish()
}

/// INPUT from bytes
pub fn read_input(b: &[u8]) -> Option<InputMsg> {
    if msg_type(b)? != MSG_INPUT {
        return None;
    }
    let mut r = BitReader::new(&b[1..]);
    let ack = if r.bool() { Some(r.u32()) } else { None };
    let full = if r.bool() { Some(r.u32()) } else { None };
    let (newest, inputs) = read_inputs(&mut r)?;
    r.ok().then_some(InputMsg { newest, inputs, ack, full })
}

/// the header of a SNAPSHOT (`kind` MSG_SNAPSHOT) or FULL (MSG_FULL); the entities follow in the same
/// writer (snapshot::encode)
pub fn write_snap_header(kind: u8, h: &SnapHeader) -> BitWriter {
    let mut w = BitWriter::with_prefix(&[kind]);
    w.u32(h.tick);
    match h.base {
        Base::None => w.bits(0, 2),
        Base::Map => w.bits(1, 2),
        Base::Back(k) => {
            w.bits(2, 2);
            w.u8(k);
        }
    }
    w.u16(h.player);
    w.svar(h.have.wrapping_sub(h.tick) as i32);
    w.bool(h.margin.is_some());
    if let Some(m) = h.margin {
        w.u8(m as u8);
    }
    w
}

/// the header of a SNAPSHOT or FULL and the reader at its entities
pub fn read_snap_header(b: &[u8]) -> Option<(u8, SnapHeader, BitReader<'_>)> {
    let kind = msg_type(b)?;
    if kind != MSG_SNAPSHOT && kind != MSG_FULL {
        return None;
    }
    let mut r = BitReader::new(&b[1..]);
    let tick = r.u32();
    let base = match r.bits(2) {
        0 => Base::None,
        1 => Base::Map,
        2 => match r.u8() {
            0 => return None,
            k => Base::Back(k),
        },
        _ => return None,
    };
    let player = r.u16();
    let have = tick.wrapping_add(r.svar() as u32);
    let margin = if r.bool() { Some(r.u8() as i8) } else { None };
    r.ok().then_some((kind, SnapHeader { tick, base, player, have, margin }, r))
}

/// FETCH as bytes
pub fn write_fetch(ticket: u64) -> Vec<u8> {
    let mut v = alloc::vec![MSG_FETCH];
    v.extend_from_slice(&ticket.to_le_bytes());
    v
}

/// FETCH's ticket
pub fn read_fetch(b: &[u8]) -> Option<u64> {
    (b.len() == 9 && b[0] == MSG_FETCH).then(|| u64::from_le_bytes([b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8]]))
}
