// addrsel -- destination address selection after RFC 6724: the scope of an address, the
// default policy table (precedence and label) and the order in which the addresses a name
// resolves to are tried (what getaddrinfo sorts), given the source address each one would use
//
//   scope(a) -> u8            RFC 6724 3.1 and 3.2: the scope of an IPv6 multicast address,
//                             2 (link-local) for fe80::/10 and ::1, 5 for the old site-local
//                             fec0::/10, 14 (global) for the other unicast addresses (unique
//                             local ones included); IPv4: 127/8 and 169.254/16 link-local, the
//                             rest global (private addresses too)
//   precedence(a) / label(a)  the default policy table of RFC 6724 2.1 (an IPv4 address as
//                             ::ffff:a.b.c.d)
//   Dest { addr, src, src_len, src_deprecated }   a destination and what source address
//                             selection gave it (src None: unusable, no route or no source)
//   compare(a, b) / sort(&mut [Dest])   the rules of RFC 6724 6: 1 (unusable last), 2 (matching
//                             scope), 3 (no deprecated source), 5 (matching label), 6 (higher
//                             precedence), 8 (smaller scope), 9 (longest matching prefix, as
//                             glibc only between two IPv6 destinations), 10 (else the order as
//                             given: the sort is stable)
//
// Rules 4 (home addresses) and 7 (native transport) decide nothing without Mobile IPv6 and
// tunnels and are left out.  The default table makes 2001:db8::/32 and other global IPv6
// destinations (precedence 40) come before IPv4 ones (35), and IPv4 before unique local IPv6
// addresses (3): a host with a global IPv6 address tries IPv6 first, one with only a link-local
// IPv6 address IPv4 first (rule 2).

use core::cmp::Ordering;

use crate::addr::{IpAddr, Ipv4, Ipv6};

/// RFC 6724 3.1: link-local scope (fe80::/10, ::1, IPv4 127/8 and 169.254/16)
pub const SCOPE_LINK: u8 = 2;
/// the deprecated site-local fec0::/10
pub const SCOPE_SITE: u8 = 5;
/// global scope
pub const SCOPE_GLOBAL: u8 = 14;

/// the policy table of RFC 6724 2.1: (prefix, length, precedence, label)
const POLICY: [([u8; 16], u8, u8, u8); 9] = [
    ([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 128, 50, 0),
    ([0; 16], 0, 40, 1),
    ([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0, 0, 0], 96, 35, 4),
    ([0x20, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 16, 30, 2),
    ([0x20, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 32, 5, 5),
    ([0xFC, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 7, 3, 13),
    ([0; 16], 96, 1, 3),
    ([0xFE, 0xC0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 10, 1, 11),
    ([0x3F, 0xFE, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 16, 1, 12),
];

/// the address as IPv6 (an IPv4 address mapped, RFC 6724 2.1)
fn as_v6(a: IpAddr) -> Ipv6 {
    match a {
        IpAddr::V4(x) => x.to_ipv6_mapped(),
        IpAddr::V6(x) => x,
    }
}

/// the entry of the policy table with the longest prefix that holds the address
fn policy(a: IpAddr) -> (u8, u8) {
    let v = as_v6(a).to_u128();
    let mut best: Option<(u8, u8, u8)> = None;
    for (p, len, prec, label) in POLICY {
        let mask = if len == 0 { 0 } else { u128::MAX << (128 - len as u32) };
        if v & mask == u128::from_be_bytes(p) & mask && best.is_none_or(|b| len > b.0) {
            best = Some((len, prec, label));
        }
    }
    best.map(|b| (b.1, b.2)).unwrap_or((40, 1))
}

/// the precedence of the default policy table (higher is preferred)
pub fn precedence(a: IpAddr) -> u8 {
    policy(a).0
}

/// the label of the default policy table (a source and a destination with the same label
/// belong together)
pub fn label(a: IpAddr) -> u8 {
    policy(a).1
}

fn scope4(a: Ipv4) -> u8 {
    if a.is_loopback() || a.is_link_local() { SCOPE_LINK } else { SCOPE_GLOBAL }
}

/// the scope of an address (RFC 6724 3.1, 3.2)
pub fn scope(a: IpAddr) -> u8 {
    match a {
        IpAddr::V4(x) => scope4(x),
        IpAddr::V6(x) => {
            if let Some(m) = x.to_ipv4_mapped() {
                scope4(m)
            } else if x.is_multicast() {
                x.mc_scope()
            } else if x.is_link_local() || x.is_loopback() {
                SCOPE_LINK
            } else if x.0[0] == 0xFE && x.0[1] & 0xC0 == 0xC0 {
                SCOPE_SITE
            } else {
                SCOPE_GLOBAL
            }
        }
    }
}

/// a destination to sort and the source address it would leave with
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dest {
    pub addr: IpAddr,
    /// the source address selection gave it; None: it cannot be reached (no route, no source)
    pub src: Option<IpAddr>,
    /// the prefix length of the source (the part before its interface identifier: rule 9
    /// counts common bits up to it)
    pub src_len: u8,
    /// the source is deprecated (its preferred lifetime ended)
    pub src_deprecated: bool,
}

impl Dest {
    pub fn new(addr: IpAddr, src: Option<IpAddr>) -> Dest {
        let src_len = match src {
            Some(IpAddr::V4(_)) => 32,
            _ => 64,
        };
        Dest { addr, src, src_len, src_deprecated: false }
    }
}

/// the bits two IPv6 addresses share, at most `cap`
fn common(a: Ipv6, b: Ipv6, cap: u8) -> u8 {
    a.common_prefix(&b).min(cap)
}

/// which of two destinations comes first (Less: `a`); Equal leaves their order
pub fn compare(a: &Dest, b: &Dest) -> Ordering {
    // rule 1: avoid unusable destinations
    let (sa, sb) = match (a.src, b.src) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Greater,
        (Some(_), None) => return Ordering::Less,
        (Some(x), Some(y)) => (x, y),
    };
    // rule 2: prefer matching scope
    let ma = scope(a.addr) == scope(sa);
    let mb = scope(b.addr) == scope(sb);
    if ma != mb {
        return if ma { Ordering::Less } else { Ordering::Greater };
    }
    // rule 3: avoid deprecated addresses
    if a.src_deprecated != b.src_deprecated {
        return if a.src_deprecated { Ordering::Greater } else { Ordering::Less };
    }
    // rule 5: prefer matching label
    let la = label(a.addr) == label(sa);
    let lb = label(b.addr) == label(sb);
    if la != lb {
        return if la { Ordering::Less } else { Ordering::Greater };
    }
    // rule 6: prefer higher precedence
    let (pa, pb) = (precedence(a.addr), precedence(b.addr));
    if pa != pb {
        return pb.cmp(&pa);
    }
    // rule 8: prefer smaller scope
    let (ca, cb) = (scope(a.addr), scope(b.addr));
    if ca != cb {
        return ca.cmp(&cb);
    }
    // rule 9: longest matching prefix, between IPv6 destinations only
    if let (IpAddr::V6(da), IpAddr::V6(db), IpAddr::V6(xa), IpAddr::V6(xb)) = (a.addr, b.addr, sa, sb) {
        let (na, nb) = (common(xa, da, a.src_len), common(xb, db, b.src_len));
        if na != nb {
            return nb.cmp(&na);
        }
    }
    // rule 10: leave the order unchanged
    Ordering::Equal
}

/// sorts destinations by RFC 6724 (stable: rule 10 keeps the given order)
pub fn sort(d: &mut [Dest]) {
    d.sort_by(compare);
}
