// hashringt -- checks of base::hashring: hash64 reference values (FNV-1a with the SplitMix64
// finaliser), owners and preference lists, the shares of the members, that adding a member
// moves keys only to it and removing it moves them back, skipping members

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::hashring::{hash64, Ring};

use crate::T;

fn ring(members: &[&str], vnodes: u32) -> Ring<String> {
    let mut r = Ring::new();
    for m in members {
        r.add(String::from(*m), m, vnodes);
    }
    r
}

pub fn run(t: &mut T) {
    t.eq("hash64 empty", hash64(b""), 0xF52A_15E9_A9B5_E89B);
    t.eq("hash64 a", hash64(b"a"), 0x02C0_BDBF_4814_20F8);
    t.eq("hash64 foobar", hash64(b"foobar"), 0x404D_A9E3_B740_78C2);
    t.eq("hash64 token", hash64(b"10.0.4.11#0"), 0x0FB5_2DAB_029B_9205);
    let empty: Ring<String> = Ring::new();
    t.check("empty ring", empty.owner(5).is_none() && empty.preference(5, 2).is_empty() && empty.is_empty());
    let names = ["10.0.4.11", "10.0.4.12", "10.0.4.13"];
    let r = ring(&names, 64);
    t.eq("tokens", r.tokens().len(), 192);
    t.check("tokens sorted", r.tokens().windows(2).all(|w| w[0].0 <= w[1].0));
    t.eq("members sorted", r.members().to_vec(), names.iter().map(|s| String::from(*s)).collect::<Vec<_>>());
    // the token of vnode 0 of a member is the hash of "key#0"
    t.check("token of vnode 0", r.tokens().iter().any(|(h, m)| *h == 0x0FB5_2DAB_029B_9205 && m == "10.0.4.11"));
    let shares = r.shares();
    let sum: u32 = shares.iter().map(|s| s.1).sum();
    t.check(&format!("shares sum to a million ({})", sum), (999_990..=1_000_000).contains(&sum));
    t.check(&format!("shares even enough {:?}", shares), shares.iter().all(|s| (250_000..420_000).contains(&s.1)));
    let one = ring(&["a"], 1);
    t.eq("one token owns all", one.shares(), alloc::vec![(String::from("a"), 1_000_000)]);
    // the owner is the first of the preference list; lists are distinct
    let keys: Vec<u64> = (0..1000).map(|i| hash64(format!("key{}", i).as_bytes())).collect();
    t.check("owner = first preference", keys.iter().all(|&h| r.owner(h) == r.preference(h, 1).first()));
    t.check("preference distinct", keys.iter().all(|&h| {
        let p = r.preference(h, 3);
        p.len() == 3 && p[0] != p[1] && p[1] != p[2] && p[0] != p[2]
    }));
    t.check("preference capped by members", r.preference(keys[0], 5).len() == 3);
    // every member owns some keys
    let mut per = [0u32; 3];
    for &h in &keys {
        if let Some(m) = r.owner(h)
            && let Some(i) = names.iter().position(|n| n == m)
        {
            per[i] += 1;
        }
    }
    t.check(&format!("keys spread {:?}", per), per.iter().all(|&c| (200..470).contains(&c)));
    // a fourth member takes keys only for itself
    let mut r4 = r.clone();
    r4.add(String::from("10.0.4.14"), "10.0.4.14", 64);
    let moved: Vec<&u64> = keys.iter().filter(|&&h| r.owner(h) != r4.owner(h)).collect();
    t.check(&format!("moved keys go to the new member ({})", moved.len()), !moved.is_empty() && moved.iter().all(|&&h| r4.owner(h).is_some_and(|m| m == "10.0.4.14")));
    t.check("about a quarter moved", (150..350).contains(&moved.len()));
    let mut back = r4.clone();
    t.check("remove", back.remove(&String::from("10.0.4.14")) && !back.remove(&String::from("10.0.4.14")));
    t.eq("removing restores the ring", back, r.clone());
    // skipping a member is the ring without it
    let skip = String::from("10.0.4.12");
    let mut without = r.clone();
    without.remove(&skip);
    t.check("preference_by skips", keys.iter().all(|&h| r.preference_by(h, 2, |m| *m != skip) == without.preference(h, 2)));
    // adding again keeps one set of tokens
    let mut again = r.clone();
    again.add(String::from("10.0.4.11"), "10.0.4.11", 64);
    t.eq("re-adding replaces", again, r);
    t.check("wraps after the last token", {
        let last = ring(&names, 4);
        let top = last.tokens().last().map(|x| x.0).unwrap_or(0);
        top < u64::MAX && last.owner(top + 1) == last.tokens().first().map(|x| &x.1)
    });
}
