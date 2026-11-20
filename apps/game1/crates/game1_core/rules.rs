// rules -- the numbers of game 1 in one place: the tick, the buttons of NetInput, the weapon, health and
// respawn, the replicated entities (their kinds and fields), the events (kill feed, hit marker, players coming
// and going: game::net's reliable events), the netchan protocol id, the bots' names.
// The feel of the movement is physics::CharacterParams (movement.rs); everything here is a start value the
// user tunes by hand (docs/GAME1.md lists them).

use alloc::string::String;
use alloc::vec::Vec;

use game::net::snapshot::Schema;

/// the working name of the game in the UI (the user picks the real one later: GAME_NAME is the one place)
pub const GAME_NAME: &str = "Spiel 1";
/// netchan's protocol id of game 1 ("G1", version 2 since the events of G7e part 2): a client of another game or
/// version is refused
pub const PROTOCOL: u32 = 0x4731_0002;
/// the WebSocket path of the server (and of the browser's client: the page's own server)
pub const WS_PATH: &str = "/game1";
/// the server's port unless set (UDP and the WebSocket's TCP)
pub const DEFAULT_PORT: u16 = 27_960;

/// ticks a second (the server's and the clients' simulation)
pub const RATE: u32 = 60;
/// the lag compensation's limit in ticks: 300 ms at 60 Hz (ServerOptions::max_rewind_ms, the user's decision of 2026-11-19)
pub const MAX_REWIND: u32 = 18;
/// figures in a room at most (players and bots)
pub const MAX_FIGURES: usize = 16;

/// a button of NetInput: jump (held: again on every landing, the controller's rule)
pub const BTN_JUMP: u16 = 1;
/// a button: walk (the quiet gait; running is the default)
pub const BTN_WALK: u16 = 2;
/// a button: fire (held: as fast as the weapon allows)
pub const BTN_FIRE: u16 = 4;
/// a button: reload (a press)
pub const BTN_RELOAD: u16 = 8;
/// a button: crouch (held)
pub const BTN_CROUCH: u16 = 16;

/// a figure's health
pub const HEALTH: i32 = 100;
/// a hit's damage on the body
pub const DAMAGE: i32 = 18;
/// a hit's damage on the head (the capsule's upper half sphere)
pub const HEAD_DAMAGE: i32 = 36;
/// rounds in the magazine (the reserve has no end in part 1)
pub const MAGAZINE: u32 = 20;
/// ticks from one shot to the next: 7 at 60 Hz, 8.6 shots a second
pub const FIRE_TICKS: u32 = 7;
/// ticks a reload takes: 1.4 s
pub const RELOAD_TICKS: u32 = 84;
/// how far a shot reaches (m)
pub const RANGE: f64 = 150.0;
/// ticks a dead figure waits before it comes back: 2 s
pub const RESPAWN_TICKS: u32 = 120;
/// ticks a figure that came back cannot be hurt (until it shoots): 1.5 s
pub const PROTECT_TICKS: u32 = 90;
/// below this height a figure has fallen out of the map (m)
pub const KILL_Y: f64 = -20.0;
/// the eye below the top of the capsule (m)
pub const EYE_BELOW_TOP: f64 = 0.15;

/// the kind of a figure
pub const KIND_FIG: u8 = 1;
/// the kind of a name tag: a figure's name, which never changes while it plays (16 fields, 60 bytes of UTF-8)
pub const KIND_TAG: u8 = 2;
/// a figure's fields: the movement's 8 (movement::MoveState), then F_YAW ..= F_INFO
pub const FIG_FIELDS: usize = 14;
/// a tag's fields: the figure's id, then 15 x 4 bytes of the name
pub const TAG_FIELDS: usize = 16;
/// the field of a figure: the view's yaw (quant::angle_q)
pub const F_YAW: usize = 8;
/// the view's pitch (quant::pitch_q)
pub const F_PITCH: usize = 9;
/// health 0 ..= HEALTH
pub const F_HEALTH: usize = 10;
/// figures it ended
pub const F_KILLS: usize = 11;
/// how often it was ended
pub const F_DEATHS: usize = 12;
/// bit 17: it cannot be hurt now (just came back), bits 18-23: the spawn point's direction (hits, kills and who
/// ended whom are events since G7e part 2: EV_KILL, EV_HIT)
pub const F_INFO: usize = 13;
/// a tag's net id: TAG_BASE + the figure's id (figures have 1 ..= 255)
pub const TAG_BASE: u16 = 512;

/// the info field: it cannot be hurt now
pub const INFO_PROTECTED: i32 = 1 << 17;
/// bits 18-23 of the info field: the yaw of the spawn point it came back at, in 64ths of a turn (the client turns
/// its camera that way once per life)
pub const INFO_SPAWN_SHIFT: u32 = 18;

/// the spawn point's yaw of an info field (radians)
pub fn info_spawn_yaw(info: i32) -> f64 {
    let k = (info >> INFO_SPAWN_SHIFT) & 63;
    let a = k as f64 * (core::f64::consts::TAU / 64.0);
    game::camera::wrap_angle(a)
}

/// a yaw (radians) as the 6 bits of the info field
pub fn spawn_dir(yaw: f64) -> i32 {
    let t = yaw / core::f64::consts::TAU * 64.0;
    let k = game::net::quant::round_i64(t);
    (k.rem_euclid(64)) as i32
}

/// the replicated entities of game 1
pub fn schema() -> Schema {
    Schema::new().with(KIND_FIG, FIG_FIELDS).with(KIND_TAG, TAG_FIELDS)
}

/// the channel of an event: a figure was ended (to everybody in the room)
pub const EV_KILL: u8 = 1;
/// the channel of an event: your shot hit (to the shooter: the hit marker)
pub const EV_HIT: u8 = 2;
/// the channel of an event: a player came or went (to everybody)
pub const EV_ROOM: u8 = 3;

/// game 1's events (game::net::event: each delivered once, in order)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameEvent {
    /// `killer` ended `victim` (killer == victim: it fell out of the map)
    Kill { killer: u16, victim: u16, head: bool },
    /// the receiver's shot hit `victim` (on the head; and ended it)
    Hit { victim: u16, head: bool, kill: bool },
    /// a figure came into the room
    Joined { id: u16, name: String },
    /// a figure left the room (or its connection ended)
    Left { id: u16, name: String },
}

impl GameEvent {
    /// the channel and the bytes
    pub fn encode(&self) -> (u8, Vec<u8>) {
        match self {
            GameEvent::Kill { killer, victim, head } => (EV_KILL, alloc::vec![*killer as u8, *victim as u8, *head as u8]),
            GameEvent::Hit { victim, head, kill } => (EV_HIT, alloc::vec![*victim as u8, *head as u8 | (*kill as u8) << 1]),
            GameEvent::Joined { id, name } | GameEvent::Left { id, name } => {
                let mut v = alloc::vec![matches!(self, GameEvent::Left { .. }) as u8, *id as u8];
                let mut cut = name.len().min(60);
                while !name.is_char_boundary(cut) {
                    cut -= 1;
                }
                v.extend_from_slice(&name.as_bytes()[..cut]);
                (EV_ROOM, v)
            }
        }
    }

    /// from a channel and bytes (None: not one of game 1's)
    pub fn decode(channel: u8, d: &[u8]) -> Option<GameEvent> {
        match (channel, d) {
            (EV_KILL, [k, v, h]) => Some(GameEvent::Kill { killer: *k as u16, victim: *v as u16, head: *h != 0 }),
            (EV_HIT, [v, f]) => Some(GameEvent::Hit { victim: *v as u16, head: f & 1 != 0, kill: f & 2 != 0 }),
            (EV_ROOM, [kind, id, name @ ..]) if *kind <= 1 => {
                let name = String::from_utf8_lossy(name).into_owned();
                Some(if *kind == 0 { GameEvent::Joined { id: *id as u16, name } } else { GameEvent::Left { id: *id as u16, name } })
            }
            _ => None,
        }
    }
}

/// the bots' names: made up, nobody's
pub const BOT_NAMES: [&str; 16] = [
    "Kiesel", "Pfeffer", "Brummer", "Zwirbel", "Knopf", "Wusel", "Socke", "Krümel", "Fussel", "Rübe", "Quirl", "Schnuppe", "Tapsi", "Hummel", "Flitz", "Murmel",
];

/// a name as the 15 fields of a tag (UTF-8, cut at a character's edge to 60 bytes)
pub fn name_fields(name: &str, out: &mut [i32]) {
    let mut cut = name.len().min(60);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    let b = &name.as_bytes()[..cut];
    for (k, o) in out.iter_mut().enumerate().take(15) {
        let mut w = [0u8; 4];
        for (j, x) in w.iter_mut().enumerate() {
            *x = b.get(k * 4 + j).copied().unwrap_or(0);
        }
        *o = i32::from_le_bytes(w);
    }
}

/// the name of a tag's 15 fields (what is not UTF-8 is left out)
pub fn fields_name(f: &[i32]) -> alloc::string::String {
    let mut b = alloc::vec::Vec::with_capacity(60);
    for v in f.iter().take(15) {
        b.extend_from_slice(&v.to_le_bytes());
    }
    while b.last() == Some(&0) {
        b.pop();
    }
    alloc::string::String::from_utf8_lossy(&b).into_owned()
}
