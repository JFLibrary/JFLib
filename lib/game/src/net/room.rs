// room -- rooms by a code or a link, players by a name, no accounts
//
//   RoomCode       six characters of an alphabet without look-alikes (no 0 O 1 I L: 31 characters, about
//                  887 million codes); random from the server's generator, parse accepts any case and
//                  ignores spaces and dashes ("k7m-2qx")
//   room_link      a link to a room: the page's address with "#CODE" (the fragment never reaches a server
//                  log); room_from_link takes a code back out of such a link, "?room=CODE", ".../r/CODE"
//                  or a bare code
//   clean_name     a player's name: trimmed, inner runs of white space as one space, no control or
//                  direction-changing characters, 1 to 16 characters; unique_name appends " 2", " 3" ...
//                  when the room has it already (compared without case)

use alloc::string::String;
use alloc::vec::Vec;

/// the characters of a room code
pub const CODE_ALPHABET: &[u8; 31] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/// characters a name may have at most
pub const MAX_NAME: usize = 16;

/// a room's code (see the module header)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoomCode(pub [u8; 6]);

impl RoomCode {
    /// a code from 64 random bits (the server's generator)
    pub fn from_random(mut r: u64) -> RoomCode {
        let mut c = [0u8; 6];
        for x in c.iter_mut() {
            *x = CODE_ALPHABET[(r % 31) as usize];
            r /= 31;
        }
        RoomCode(c)
    }

    /// a code typed by a person: any case, spaces and dashes ignored; None unless six characters of the
    /// alphabet remain
    pub fn parse(s: &str) -> Option<RoomCode> {
        let mut c = [0u8; 6];
        let mut n = 0;
        for ch in s.bytes() {
            if ch == b' ' || ch == b'-' {
                continue;
            }
            let u = ch.to_ascii_uppercase();
            if !CODE_ALPHABET.contains(&u) || n == 6 {
                return None;
            }
            c[n] = u;
            n += 1;
        }
        (n == 6).then_some(RoomCode(c))
    }

    /// the six characters
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).unwrap_or("??????")
    }

    /// the code's bytes are all of the alphabet (a code read from the network)
    pub fn valid(&self) -> bool {
        self.0.iter().all(|c| CODE_ALPHABET.contains(c))
    }
}

impl core::fmt::Display for RoomCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// a link to a room: `base` (the page, such as "https://spiel.example/") with "#CODE"
pub fn room_link(base: &str, code: &RoomCode) -> String {
    let base = base.split('#').next().unwrap_or(base);
    let mut s = String::from(base);
    s.push('#');
    s.push_str(code.as_str());
    s
}

/// the code of a link (fragment "#CODE", query "room=CODE", a path ending in "/r/CODE") or a bare code
pub fn room_from_link(link: &str) -> Option<RoomCode> {
    let link = link.trim();
    if let Some(c) = RoomCode::parse(link) {
        return Some(c);
    }
    if let Some((_, frag)) = link.split_once('#') {
        if let Some(c) = RoomCode::parse(frag) {
            return Some(c);
        }
    }
    let before_frag = link.split('#').next().unwrap_or(link);
    if let Some((_, q)) = before_frag.split_once('?') {
        for kv in q.split('&') {
            if let Some(v) = kv.strip_prefix("room=") {
                return RoomCode::parse(v);
            }
        }
    }
    let path = before_frag.split('?').next().unwrap_or(before_frag);
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 && parts[parts.len() - 2] == "r" {
        return RoomCode::parse(parts[parts.len() - 1]);
    }
    None
}

// characters a name never has: controls, the direction marks and overrides, the zero-width ones, the
// replacement character
fn forbidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fffd}' | '\u{fff9}'..='\u{fffb}')
}

/// a player's name as the room shows it (see the module header); None when nothing is left
pub fn clean_name(raw: &str) -> Option<String> {
    let mut s = String::new();
    let mut space = false;
    let mut n = 0;
    for c in raw.chars() {
        if c.is_whitespace() {
            space = !s.is_empty();
            continue;
        }
        if forbidden(c) {
            continue;
        }
        if n == MAX_NAME {
            break;
        }
        if space {
            if n + 1 >= MAX_NAME {
                break;
            }
            s.push(' ');
            n += 1;
            space = false;
        }
        s.push(c);
        n += 1;
    }
    (!s.is_empty()).then_some(s)
}

/// `name` made unique among `taken` (compared ASCII-case-insensitively): " 2", " 3" ... appended, the
/// name shortened to keep MAX_NAME characters
pub fn unique_name(name: &str, taken: &[&str]) -> String {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    if !taken.iter().any(|t| same(t, name)) {
        return String::from(name);
    }
    for k in 2..1000u32 {
        let suffix = alloc::format!(" {}", k);
        let keep = MAX_NAME - suffix.len();
        let mut s: String = name.chars().take(keep).collect();
        s.push_str(&suffix);
        if !taken.iter().any(|t| same(t, &s)) {
            return s;
        }
    }
    String::from(name)
}
