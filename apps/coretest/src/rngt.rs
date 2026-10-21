// rngt -- checks of base::rng: the reference outputs of PCG32 (pcg32-demo, seed 42 stream 54)
// and SplitMix64, jump ahead, bounds and determinism

use alloc::vec::Vec;

use base::rng::{Pcg32, SplitMix64};

use crate::T;

pub fn run(t: &mut T) {
    let mut g = Pcg32::new(42, 54);
    let first: Vec<u32> = (0..6).map(|_| g.next_u32()).collect();
    t.eq("pcg32 reference", first, alloc::vec![0xA15C_02B7, 0x7B47_F409, 0xBA1D_3330, 0x83D2_F293, 0xBFA4_784B, 0xCBED_606E]);
    let mut s = SplitMix64::new(0);
    t.eq("splitmix64 seed 0", [s.next_u64(), s.next_u64(), s.next_u64()], [0xE220_A839_7B1D_CDAF, 0x6E78_9E6A_A1B9_65F4, 0x06C4_5D18_8009_454F]);
    t.eq("splitmix64 seed 1234567", SplitMix64::new(1_234_567).next_u64(), 6_457_827_717_110_365_317);
    // jump ahead equals stepping
    let mut a = Pcg32::new(7, 3);
    let mut b = a.clone();
    for _ in 0..1000 {
        a.next_u32();
    }
    b.advance(1000);
    t.eq("advance 1000", a.next_u32(), b.next_u32());
    // bounds
    let mut g = Pcg32::new(1, 1);
    let mut counts = [0u32; 10];
    for _ in 0..10_000 {
        counts[g.below(10) as usize] += 1;
    }
    t.check("below uniform-ish", counts.iter().all(|&c| (850..1150).contains(&c)));
    t.eq("below 0", g.below(0), 0);
    t.check("range inclusive", (0..1000).all(|_| (-3..=3).contains(&g.range(-3, 3))));
    t.eq("range empty", g.range(5, 5), 5);
    let _ = g.range(i64::MIN, i64::MAX);
    t.check("range wide", (0..100).all(|_| (0..=(1i64 << 40)).contains(&g.range(0, 1 << 40))));
    t.check("f64 unit", (0..1000).all(|_| (0.0..1.0).contains(&g.f64())));
    t.check("chance edges", g.chance(5, 5) && !g.chance(0, 5) && g.chance_ppm(1_000_000) && !g.chance_ppm(0));
    // determinism and independence of derived generators
    let take4 = |mut g: Pcg32| -> Vec<u32> { (0..4).map(|_| g.next_u32()).collect() };
    let x = take4(Pcg32::derive(99, 5));
    let y = take4(Pcg32::derive(99, 5));
    let z = take4(Pcg32::derive(99, 6));
    t.check("derive deterministic", x == y);
    t.check("derive independent", x != z);
    let mut v: Vec<u32> = (0..50).collect();
    g.shuffle(&mut v);
    let mut w = v.clone();
    w.sort();
    t.check("shuffle permutes", w == (0..50).collect::<Vec<u32>>() && v != w);
    let mut buf = [0u8; 7];
    g.fill(&mut buf);
    t.check("fill", buf.iter().any(|&b| b != 0));
}
