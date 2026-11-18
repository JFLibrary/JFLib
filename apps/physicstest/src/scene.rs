// scene -- meshes for the tests, the checksums and the measurements: hilly ground from integer noise,
// boxes, steps and ramps on it (convex and concave edges), a flat grid for the seams.  Integer hashing and
// f32 products only: the same vertices on every target.

use alloc::vec::Vec;

pub struct MeshData {
    pub pos: Vec<[f32; 3]>,
    pub idx: Vec<u32>,
}

impl MeshData {
    pub fn new() -> MeshData {
        MeshData { pos: Vec::new(), idx: Vec::new() }
    }

    pub fn triangles(&self) -> usize {
        self.idx.len() / 3
    }

    pub fn build(&self) -> physics::TriMesh {
        physics::TriMesh::new(&self.pos, &self.idx).expect("a valid mesh")
    }
}

fn hash(i: i32, j: i32, seed: u32) -> u32 {
    let mut h = (i as u32).wrapping_mul(0x8da6_b343) ^ (j as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    h
}

/// ground of n x n quads of `cell` metres from (0, 0), heights from smoothed integer noise up to `amp`;
/// each quad split along a diagonal that alternates (so vertices have 4 and 8 neighbours)
pub fn ground(m: &mut MeshData, n: usize, cell: f32, amp: f32, seed: u32) {
    let base = m.pos.len() as u32;
    let raw = |i: i32, j: i32| (hash(i, j, seed) % 1024) as f32 / 1024.0;
    for j in 0..=n as i32 {
        for i in 0..=n as i32 {
            let mut h = 0.0f32;
            for (di, dj, w) in [(0, 0, 4.0f32), (1, 0, 2.0), (-1, 0, 2.0), (0, 1, 2.0), (0, -1, 2.0), (1, 1, 1.0), (-1, -1, 1.0), (1, -1, 1.0), (-1, 1, 1.0)] {
                h += raw(i + di, j + dj) * w;
            }
            m.pos.push([i as f32 * cell, h / 16.0 * amp, j as f32 * cell]);
        }
    }
    let w = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let (a, b, c, d) = (base + j * w + i, base + j * w + i + 1, base + (j + 1) * w + i + 1, base + (j + 1) * w + i);
            if (i + j) % 2 == 0 {
                m.idx.extend_from_slice(&[a, d, c, a, c, b]);
            } else {
                m.idx.extend_from_slice(&[a, d, b, b, d, c]);
            }
        }
    }
}

/// a box (12 triangles, outward winding)
pub fn cuboid(m: &mut MeshData, c: [f32; 3], h: [f32; 3]) {
    let base = m.pos.len() as u32;
    for k in 0..8 {
        m.pos.push([
            c[0] + if k & 1 != 0 { h[0] } else { -h[0] },
            c[1] + if k & 2 != 0 { h[1] } else { -h[1] },
            c[2] + if k & 4 != 0 { h[2] } else { -h[2] },
        ]);
    }
    // the faces counter-clockwise from outside
    let faces: [[u32; 4]; 6] = [[0, 4, 6, 2], [1, 3, 7, 5], [0, 1, 5, 4], [2, 6, 7, 3], [0, 2, 3, 1], [4, 5, 7, 6]];
    for f in faces {
        m.idx.extend_from_slice(&[base + f[0], base + f[1], base + f[2], base + f[0], base + f[2], base + f[3]]);
    }
}

/// a ramp: a quad from (x0, y0, z0) rising to height `rise` over `len` along +x, `wide` along z
pub fn ramp(m: &mut MeshData, p: [f32; 3], len: f32, wide: f32, rise: f32) {
    let base = m.pos.len() as u32;
    m.pos.push(p);
    m.pos.push([p[0] + len, p[1] + rise, p[2]]);
    m.pos.push([p[0] + len, p[1] + rise, p[2] + wide]);
    m.pos.push([p[0], p[1], p[2] + wide]);
    m.idx.extend_from_slice(&[base, base + 3, base + 2, base, base + 2, base + 1]);
}

/// the map of the tests: n x n ground, a field of boxes, steps and ramps
pub fn map(n: usize, boxes: usize) -> MeshData {
    let mut m = MeshData::new();
    let cell = 1.0f32;
    ground(&mut m, n, cell, 1.5, 7);
    let size = n as f32 * cell;
    for k in 0..boxes {
        let hx = (hash(k as i32, 1, 99) % 100) as f32 / 100.0 * 1.5 + 0.2;
        let hy = (hash(k as i32, 2, 99) % 100) as f32 / 100.0 * 2.0 + 0.1;
        let hz = (hash(k as i32, 3, 99) % 100) as f32 / 100.0 * 1.5 + 0.2;
        let x = (hash(k as i32, 4, 99) % 10_000) as f32 / 10_000.0 * size;
        let z = (hash(k as i32, 5, 99) % 10_000) as f32 / 10_000.0 * size;
        cuboid(&mut m, [x, hy + 0.5, z], [hx, hy, hz]);
    }
    // steps: 8 boxes rising by 0.2 m
    for s in 0..8 {
        let x = size * 0.25 + s as f32 * 0.4;
        cuboid(&mut m, [x, 1.6 + 0.1 * s as f32, size * 0.5], [0.2, 0.1 * (s + 1) as f32 + 1.5 * 0.5, 2.0]);
    }
    ramp(&mut m, [size * 0.6, 1.8, size * 0.3], 6.0, 3.0, 2.5);
    m
}

/// a flat grid of n x n quads of `cell` metres at height y, alternating diagonals
pub fn flat(n: usize, cell: f32, y: f32) -> MeshData {
    let mut m = MeshData::new();
    ground(&mut m, n, cell, 0.0, 1);
    for p in &mut m.pos {
        p[1] = y;
    }
    m
}
