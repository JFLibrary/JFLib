// addrselt -- checks of netpkt::addrsel: the scopes of RFC 6724 3.1 and 3.2, the default policy
// table of 2.1 (precedence and label), and destination address selection with the examples of
// RFC 6724 10.2 (each destination with the source address the RFC gives it), unusable
// destinations last and the order kept between equals.

use alloc::vec;
use alloc::vec::Vec;

use netpkt::addrsel::{self, Dest, SCOPE_GLOBAL, SCOPE_LINK, SCOPE_SITE};
use netpkt::IpAddr;

use crate::T;

fn ip(s: &str) -> IpAddr {
    IpAddr::parse(s).unwrap_or(IpAddr::V4(netpkt::Ipv4::UNSPECIFIED))
}

/// a destination with its source address
fn d(addr: &str, src: &str) -> Dest {
    Dest::new(ip(addr), Some(ip(src)))
}

/// the order of the destinations after sorting
fn sorted(mut v: Vec<Dest>) -> Vec<IpAddr> {
    addrsel::sort(&mut v);
    v.into_iter().map(|x| x.addr).collect()
}

pub fn run(t: &mut T) {
    scopes(t);
    policy(t);
    examples(t);
    edges(t);
}

// RFC 6724 3.1 (IPv6 by its kind and the multicast scope field) and 3.2 (IPv4)
fn scopes(t: &mut T) {
    let cases: [(&str, u8); 12] = [
        ("ff02::1", 2),
        ("ff05::2", 5),
        ("ff0e::1", 14),
        ("fe80::1", SCOPE_LINK),
        ("::1", SCOPE_LINK),
        ("fec0::1", SCOPE_SITE),
        ("fd00::1", SCOPE_GLOBAL),
        ("2001:db8::1", SCOPE_GLOBAL),
        ("127.0.0.1", SCOPE_LINK),
        ("169.254.13.78", SCOPE_LINK),
        ("10.1.2.3", SCOPE_GLOBAL),
        ("::ffff:169.254.1.1", SCOPE_LINK),
    ];
    for (a, s) in cases {
        t.eq(&alloc::format!("scope of {}", a), addrsel::scope(ip(a)), s);
    }
}

// the default policy table of RFC 6724 2.1: the longest prefix decides
fn policy(t: &mut T) {
    let cases: [(&str, u8, u8); 11] = [
        ("::1", 50, 0),
        ("2001:db8:1::1", 40, 1),
        ("fe80::1", 40, 1),
        ("10.1.2.3", 35, 4),
        ("::ffff:198.51.100.121", 35, 4),
        ("2002:c633:6401::1", 30, 2),
        ("2001:0:4136:e378::1", 5, 5),
        ("fd12:3456::1", 3, 13),
        ("::10.1.2.3", 1, 3),
        ("fec0::1", 1, 11),
        ("3ffe::1", 1, 12),
    ];
    for (a, prec, label) in cases {
        t.eq(&alloc::format!("policy of {}", a), (addrsel::precedence(ip(a)), addrsel::label(ip(a))), (prec, label));
    }
}

// RFC 6724 10.2 (the home address example needs Mobile IPv6 and is left out)
fn examples(t: &mut T) {
    t.eq("matching scope (IPv6 first)", sorted(vec![d("2001:db8:1::1", "2001:db8:1::2"), d("198.51.100.121", "169.254.13.78")]), vec![ip("2001:db8:1::1"), ip("198.51.100.121")]);
    t.eq("matching scope (IPv4 first)", sorted(vec![d("2001:db8:1::1", "fe80::1"), d("198.51.100.121", "198.51.100.117")]), vec![ip("198.51.100.121"), ip("2001:db8:1::1")]);
    t.eq("higher precedence (IPv6 before IPv4)", sorted(vec![d("10.1.2.3", "10.1.2.4"), d("2001:db8:1::1", "2001:db8:1::2")]), vec![ip("2001:db8:1::1"), ip("10.1.2.3")]);
    t.eq("smaller scope", sorted(vec![d("2001:db8:1::1", "2001:db8:1::2"), d("fe80::1", "fe80::2")]), vec![ip("fe80::1"), ip("2001:db8:1::1")]);
    let mut dep = d("2001:db8:1::1", "2001:db8:1::2");
    dep.src_deprecated = true;
    t.eq("avoid deprecated sources", sorted(vec![dep, d("fe80::1", "fe80::2")]), vec![ip("fe80::1"), ip("2001:db8:1::1")]);
    t.eq("longest matching prefix", sorted(vec![d("2001:db8:3e::1", "2001:db8:3f44::2"), d("2001:db8:1::1", "2001:db8:1::2")]), vec![ip("2001:db8:1::1"), ip("2001:db8:3e::1")]);
    t.eq("matching label", sorted(vec![d("2001:db8:1::1", "2002:c633:6401::2"), d("2002:c633:6401::1", "2002:c633:6401::2")]), vec![ip("2002:c633:6401::1"), ip("2001:db8:1::1")]);
    t.eq("higher precedence (6to4 last)", sorted(vec![d("2002:c633:6401::1", "2002:c633:6401::2"), d("2001:db8:1::1", "2001:db8:1::2")]), vec![ip("2001:db8:1::1"), ip("2002:c633:6401::1")]);
}

// unusable destinations, unique local against IPv4, the cap of rule 9, ties
fn edges(t: &mut T) {
    let none = Dest::new(ip("2001:db8:9::1"), None);
    t.eq("unusable last", sorted(vec![none, d("10.0.0.1", "10.0.0.2")]), vec![ip("10.0.0.1"), ip("2001:db8:9::1")]);
    t.eq("IPv4 before unique local", sorted(vec![d("fd00::1", "fd00::2"), d("10.0.0.1", "10.0.0.2")]), vec![ip("10.0.0.1"), ip("fd00::1")]);
    // the bits after the source's /64 do not count: the order stays
    t.eq("prefix capped at the source's length", sorted(vec![d("2001:db8:1::ff", "2001:db8:1::1"), d("2001:db8:1::1:0:1", "2001:db8:1::1")]), vec![ip("2001:db8:1::ff"), ip("2001:db8:1::1:0:1")]);
    t.eq("ties keep their order", sorted(vec![d("10.0.0.9", "10.0.0.2"), d("10.0.0.1", "10.0.0.2")]), vec![ip("10.0.0.9"), ip("10.0.0.1")]);
    t.eq("compare equal", addrsel::compare(&d("10.0.0.9", "10.0.0.2"), &d("10.0.0.1", "10.0.0.2")), core::cmp::Ordering::Equal);
    t.eq("source lengths", (Dest::new(ip("10.0.0.1"), Some(ip("10.0.0.2"))).src_len, Dest::new(ip("2001:db8::1"), Some(ip("2001:db8::2"))).src_len), (32, 64));
}
