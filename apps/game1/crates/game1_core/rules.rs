// rules -- the numbers of game 1 in one place: the tick, the buttons of NetInput, the weapon, health and
// respawn, the replicated entities (their kinds and fields), the netchan protocol id, the bots' names.
// The feel of the movement is physics::CharacterParams (movement.rs); everything here is a start value the
// user tunes by hand (docs/GAME1.md lists them).

use game::net::snapshot::Schema;

/// the working name of the game in the UI (the user picks the real one later: GAME_NAME is the one place)
pub const GAME_NAME: &str = "Spiel 1";
/// netchan's protocol id of game 1 ("G1", version 1): a client of another game or version is refused
pub const PROTOCOL: u32 = 0x4731_0001;
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
/// bits 0-7: its hits (a counter: the hit marker), 8-15: who ended it last, 16: that was a head shot, 17: it cannot
/// be hurt now (just came back), 24-31: its head shots (a counter)
pub const F_INFO: usize = 13;
/// a tag's net id: TAG_BASE + the figure's id (figures have 1 ..= 255)
pub const TAG_BASE: u16 = 512;

/// the info field's parts
pub const INFO_HEAD: i32 = 1 << 16;
/// see INFO_HEAD
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

/// the hits counter of an info field
pub fn info_hits(info: i32) -> u8 {
    (info & 0xff) as u8
}

/// who ended the figure last (a figure's id; 0: nobody yet)
pub fn info_killer(info: i32) -> u16 {
    ((info >> 8) & 0xff) as u16
}

/// the head shots counter of an info field
pub fn info_heads(info: i32) -> u8 {
    ((info >> 24) & 0xff) as u8
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
