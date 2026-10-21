// ifname -- interface names the way IOS writes and reads them
//
//   short("GigabitEthernet1/0/1") -> "Gi1/0/1"      (other names unchanged)
//   expand("gi1/0/1" | "g 1/0/1" | "Gig1/0/1")   -> Some("GigabitEthernet1/0/1")
//   expand("t1/0/1") -> None (ambiguous: Ten, Two, TwentyFive, Tunnel)
//   same(a, b)                                    two spellings of one IOS interface
//
// A type prefix may be any unique prefix of the long name, or exactly one of the short
// forms ("tw" = TwoGigabitEthernet although TwentyFiveGigE starts with "tw" too).  Linux names
// are no IOS names: callers compare a name as it is first and expand only when that fails
// ("eth0" would expand to "Ethernet0", "wlan0" to nothing).

use alloc::string::String;

/// (long name, short name) of the IOS interface types
pub const TYPES: &[(&str, &str)] = &[
    ("GigabitEthernet", "Gi"),
    ("TwoGigabitEthernet", "Tw"),
    ("FiveGigabitEthernet", "Fi"),
    ("TenGigabitEthernet", "Te"),
    ("TwentyFiveGigE", "Twe"),
    ("FortyGigabitEthernet", "Fo"),
    ("HundredGigE", "Hu"),
    ("FastEthernet", "Fa"),
    ("Ethernet", "Et"),
    ("Serial", "Se"),
    ("Vlan", "Vl"),
    ("Loopback", "Lo"),
    ("Port-channel", "Po"),
    ("Tunnel", "Tu"),
    ("Dot11Radio", "Do"),
    ("Console", "Con"),
];

/// splits "GigabitEthernet1/0/1" into ("GigabitEthernet", "1/0/1"); spaces between are dropped
pub fn split(name: &str) -> (&str, &str) {
    let t = name.trim();
    let end = t.find(|c: char| c.is_ascii_digit() || c == ' ').unwrap_or(t.len());
    (&t[..end], t[end..].trim_start())
}

/// the short IOS form ("Gi1/0/1"); names of other kinds come back unchanged
pub fn short(name: &str) -> String {
    let (ty, rest) = split(name);
    for &(long, sh) in TYPES {
        if ty == long {
            let mut s = String::from(sh);
            s.push_str(rest);
            return s;
        }
    }
    String::from(name.trim())
}

/// the long IOS name of an abbreviation; None when the prefix is unknown or ambiguous
pub fn expand(input: &str) -> Option<String> {
    let (ty, rest) = split(input);
    if ty.is_empty() {
        return None;
    }
    let long = expand_type(ty)?;
    let mut s = String::from(long);
    s.push_str(rest);
    Some(s)
}

/// the long name of a type prefix ("gi" -> "GigabitEthernet")
pub fn expand_type(ty: &str) -> Option<&'static str> {
    if let Some(&(long, _)) = TYPES.iter().find(|(_, sh)| sh.eq_ignore_ascii_case(ty)) {
        return Some(long);
    }
    let mut found = None;
    for &(long, _) in TYPES {
        if long.len() >= ty.len() && long[..ty.len()].eq_ignore_ascii_case(ty) {
            if found.is_some() {
                return None;
            }
            found = Some(long);
        }
    }
    found
}

/// two spellings of the same interface ("Gi1/0/1" and "GigabitEthernet1/0/1", "eth0" and "ETH0")
pub fn same(a: &str, b: &str) -> bool {
    if a.trim().eq_ignore_ascii_case(b.trim()) {
        return true;
    }
    match (expand(a), expand(b)) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(&y),
        _ => false,
    }
}
