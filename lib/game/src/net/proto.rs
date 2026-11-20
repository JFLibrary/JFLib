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
//   LEAVE      client -> server   the player leaves: the server removes the figure at once
//
// The event part (event.rs): bit 7 of byte 0 (EXT) says that an INPUT or SNAPSHOT carries an acknowledgement of
// the other side's events and / or a run of its own events: after a SNAPSHOT's header (before the entities, so the
// budget counts it), after an INPUT's inputs.  Without events the bytes are those of version 1.
//
// Every reader returns None for anything malformed or too large (bytes from the network never panic).

use alloc::string::String;
use alloc::vec::Vec;

use super::bits::{BitReader, BitWriter};
use super::event::{read_section, write_section, NetEvent};
use super::input::{read_inputs, write_inputs, NetInput};
use super::room::RoomCode;

/// the netcode's own version (HELLO); a game's version goes into netchan's protocol id.  2 since G7e part 2 (the
/// event part, LEAVE)
pub const VERSION: u16 = 2;

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
/// see the module header
pub const MSG_LEAVE: u8 = 8;
/// bit 7 of byte 0: an event part follows (see the module header)
pub const EXT: u8 = 0x80;

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
    /// the event part: the server has every event of the client up to this one
    pub ev_ack: Option<u16>,
    /// the event part: a run of the server's events follows the header (event::read_section, ticks against `tick`)
    pub events: bool,
}

impl SnapHeader {
    /// a header without an event part
    pub fn new(tick: u32, base: Base, player: u16, have: u32, margin: Option<i8>) -> SnapHeader {
        SnapHeader { tick, base, player, have, margin, ev_ack: None, events: false }
    }
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
    /// the event part: the client has every event of the server up to this one
    pub ev_ack: Option<u16>,
    /// the event part: a run of the client's events (the number of the first and the events; ticks against `newest`)
    pub events: Option<(u16, Vec<NetEvent>)>,
}

impl InputMsg {
    /// an INPUT without an event part
    pub fn new(newest: u32, inputs: Vec<NetInput>, ack: Option<u32>, full: Option<u32>) -> InputMsg {
        InputMsg { newest, inputs, ack, full, ev_ack: None, events: None }
    }
}

/// the type of a message without the EXT flag (None: empty)
pub fn msg_type(b: &[u8]) -> Option<u8> {
    b.first().map(|t| t & !EXT)
}

/// whether a message has an event part (EXT)
pub fn has_ext(b: &[u8]) -> bool {
    b.first().is_some_and(|t| t & EXT != 0)
}

// the event part: the acknowledgement, then whether a run follows
fn write_ext(w: &mut BitWriter, ack: Option<u16>, run: bool) {
    w.bool(ack.is_some());
    if let Some(a) = ack {
        w.u16(a);
    }
    w.bool(run);
}

fn read_ext(r: &mut BitReader) -> (Option<u16>, bool) {
    let ack = if r.bool() { Some(r.u16()) } else { None };
    (ack, r.bool())
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
    let run = m.events.as_ref().is_some_and(|e| !e.1.is_empty());
    let ext = m.ev_ack.is_some() || run;
    let mut w = BitWriter::with_prefix(&[MSG_INPUT | if ext { EXT } else { 0 }]);
    w.bool(m.ack.is_some());
    if let Some(a) = m.ack {
        w.u32(a);
    }
    w.bool(m.full.is_some());
    if let Some(f) = m.full {
        w.u32(f);
    }
    write_inputs(&mut w, m.newest, &m.inputs);
    if ext {
        write_ext(&mut w, m.ev_ack, run);
        if let Some((first, ev)) = m.events.as_ref().filter(|_| run) {
            write_section(&mut w, *first, ev, m.newest);
        }
    }
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
    let (mut ev_ack, mut events) = (None, None);
    if has_ext(b) {
        let (a, run) = read_ext(&mut r);
        ev_ack = a;
        if run {
            events = Some(read_section(&mut r, newest)?);
        }
    }
    r.ok().then_some(InputMsg { newest, inputs, ack, full, ev_ack, events })
}

/// the header of a SNAPSHOT (`kind` MSG_SNAPSHOT) or FULL (MSG_FULL); the entities follow in the same
/// writer (snapshot::encode)
pub fn write_snap_header(kind: u8, h: &SnapHeader) -> BitWriter {
    let ext = h.ev_ack.is_some() || h.events;
    let mut w = BitWriter::with_prefix(&[kind | if ext { EXT } else { 0 }]);
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
    if ext {
        write_ext(&mut w, h.ev_ack, h.events);
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
    let (ev_ack, events) = if has_ext(b) { read_ext(&mut r) } else { (None, false) };
    r.ok().then_some((kind, SnapHeader { tick, base, player, have, margin, ev_ack, events }, r))
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

/// LEAVE as bytes
pub fn write_leave() -> Vec<u8> {
    alloc::vec![MSG_LEAVE]
}

/// whether the bytes are a LEAVE
pub fn is_leave(b: &[u8]) -> bool {
    b == [MSG_LEAVE]
}
