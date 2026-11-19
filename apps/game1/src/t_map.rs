// t_map -- the group "map": the baked map and figure (.g1m) read back byte for byte, their contents (spawn points,
// way points and their links, the muzzle), every spawn point free and on the ground, the way-point graph connected,
// the collision mesh's checksum and a checksum of 20 000 rays against it (the same numbers on every target), the
// reader against broken files (no panic)

use alloc::collections::VecDeque;
use alloc::format;
use alloc::vec::Vec;

use game1_core::map::{fnv, Map, ModelData, FNV0, WERFT};
use game1_core::movement::Mover;
use physics::{Filter, Iso, Vec3};

use crate::{Ctx, Rng};

/// the collision mesh's checksum of werft.g1m (docs/GAME1.md, "Prüfsummen"); a new bake changes it on purpose
pub const COLLISION_SUM: u64 = 0x2967_51e5_38ed_ff1f;

/// FNV-1a over 20 000 rays from a grid above and inside the map in random directions: hit or not, t, the normal
pub fn ray_sum(map: &Map, n: usize) -> u64 {
    let mut rng = Rng::new(77);
    let mut h = FNV0;
    for _ in 0..n {
        let o = Vec3::new(rng.range(-30.0, 30.0), rng.range(0.1, 8.0), rng.range(-22.0, 22.0));
        let (sn, cs) = physics::math::sincos(rng.range(-3.2, 3.2));
        let y = rng.range(-0.9, 0.5);
        let d = Vec3::new(sn, y, cs).normalize_or_zero();
        match map.world.ray_cast(o, d, 120.0, &Filter::ALL) {
            Some((_, hit)) => {
                h = fnv(h, &hit.t.to_bits().to_le_bytes());
                for v in [hit.normal.x, hit.normal.y, hit.normal.z] {
                    h = fnv(h, &v.to_bits().to_le_bytes());
                }
                h = fnv(h, &hit.triangle.to_le_bytes());
            }
            None => h = fnv(h, &[0xff]),
        }
    }
    h
}

pub fn run(c: &mut Ctx) {
    // read back byte for byte
    let data = match ModelData::decode(WERFT) {
        Ok(d) => d,
        Err(e) => {
            c.check(false, || format!("werft.g1m: {}", e));
            return;
        }
    };
    c.check(data.encode() == WERFT, || "werft.g1m does not encode back to the same bytes".into());
    let fig = ModelData::decode(crate::look::FIGURE);
    c.check(fig.as_ref().is_ok_and(|f| f.encode() == crate::look::FIGURE), || "figur.g1m does not read back".into());
    if let Ok(f) = &fig {
        for p in ["bein_l", "bein_r", "rumpf", "kopf", "arm_l", "arm_r", "haende", "waffe"] {
            c.check(f.part(p).is_some(), || format!("the figure has no part {}", p));
        }
        c.check(f.marker("muendung").is_some(), || "the figure has no muzzle".into());
        c.check(f.materials.iter().any(|m| m.name == "jacke"), || "the figure has no jacket".into());
    }
    let map = match Map::new(data) {
        Ok(m) => m,
        Err(e) => {
            c.check(false, || format!("the map: {}", e));
            return;
        }
    };
    c.note(&format!(
        "{} parts, {} collision triangles, {} spawn points, {} way points, {} links, collision={:016x}",
        map.data.parts.len(),
        map.triangles(),
        map.spawns.len(),
        map.points.len(),
        map.data.links.len(),
        map.collision_sum()
    ));
    if COLLISION_SUM != 0 {
        c.check(map.collision_sum() == COLLISION_SUM, || format!("collision checksum {:016x}, expected {:016x}", map.collision_sum(), COLLISION_SUM));
    }
    c.check(map.spawns.len() == 12, || format!("{} spawn points", map.spawns.len()));
    c.check(map.points.len() >= 30, || format!("{} way points", map.points.len()));
    // every spawn point: the standing capsule free, the ground under the feet, at least 4 m from the others
    let mover = Mover::default();
    let cap = mover.shape(false);
    let mut hits = Vec::new();
    for (k, s) in map.spawns.iter().enumerate() {
        let centre = s.feet + Vec3::new(0.0, 0.5 * mover.height(false) + 0.01, 0.0);
        hits.clear();
        map.world.overlap(&cap, &Iso::at(centre), &Filter::ALL, &mut hits);
        c.check(hits.is_empty(), || format!("spawn {} at {:?} is inside the map", k, s.feet));
        let ground = map.world.ray_cast(s.feet + Vec3::new(0.0, 0.2, 0.0), Vec3::new(0.0, -1.0, 0.0), 0.5, &Filter::ALL);
        c.check(ground.is_some_and(|(_, h)| h.t < 0.3), || format!("spawn {} has no ground under it", k));
        for (j, o) in map.spawns.iter().enumerate().skip(k + 1) {
            c.check((o.feet - s.feet).length() >= 4.0, || format!("spawns {} and {} are close", k, j));
        }
    }
    // the way points: each on the ground with a neighbour, the graph connected
    for (k, p) in map.points.iter().enumerate() {
        c.check(!map.neighbours[k].is_empty(), || format!("way point {} at {:?} has no neighbour", k, p));
        let ground = map.world.ray_cast(*p + Vec3::new(0.0, 0.3, 0.0), Vec3::new(0.0, -1.0, 0.0), 0.6, &Filter::ALL);
        c.check(ground.is_some(), || format!("way point {} has no ground under it", k));
    }
    let mut seen = alloc::vec![false; map.points.len()];
    let mut q = VecDeque::new();
    if !seen.is_empty() {
        seen[0] = true;
        q.push_back(0usize);
    }
    while let Some(k) = q.pop_front() {
        for &n in &map.neighbours[k] {
            if !seen[n] {
                seen[n] = true;
                q.push_back(n);
            }
        }
    }
    let reached = seen.iter().filter(|s| **s).count();
    c.check(reached == map.points.len(), || format!("the way-point graph reaches {} of {} from the first", reached, map.points.len()));
    // the rays
    let rays = ray_sum(&map, 20_000);
    c.note(&format!("rays={:016x} (20 000)", rays));
    // broken files: cut, flipped bytes, random; never a panic
    let mut rng = Rng::new(5);
    let mut ok = 0;
    for i in 0..c.n(2000) {
        let mut b = WERFT.to_vec();
        match i % 3 {
            0 => b.truncate(rng.below(b.len())),
            1 => {
                for _ in 0..1 + rng.below(8) {
                    let k = rng.below(b.len());
                    b[k] ^= 1 << rng.below(8);
                }
            }
            _ => {
                let n = rng.below(4096);
                b = (0..n).map(|_| rng.next_u64() as u8).collect();
                if n > 8 {
                    b[..4].copy_from_slice(b"G1M\x01");
                }
            }
        }
        if ModelData::decode(&b).is_ok() {
            ok += 1;
        }
    }
    c.note(&format!("broken files: {} of {} still read (flipped bits in floats), none panicked", ok, c.n(2000)));
}
