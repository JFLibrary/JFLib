// checksum -- the determinism harness: every example built from its script (JFScript's soft floats, the host
// functions, lib/geom), every part's mesh bits and the numbers of its checks, then a fixed set of geom
// operations of every kind (since G7g part 2 also Boolean operations and bevels, and the example turm.jfs), folded
// bit for bit into one FNV-1a hash.  The same number on Windows, Linux,
// macOS and the browser means the same script gives the same model there (docs/GEOM.md, "Prüfsummen").

use alloc::format;
use alloc::vec::Vec;

use geom::{Axis, Fnv, Mesh, Report, Vec3};

use crate::{examples, run};

struct Hash {
    h: Fnv,
    n: u64,
}

impl Hash {
    fn u(&mut self, v: u64) {
        self.h.u(v);
        self.n += 1;
    }

    fn f(&mut self, v: f64) {
        self.u(v.to_bits());
    }

    fn mesh(&mut self, m: &Mesh) {
        self.u(m.fingerprint());
        self.n += (m.positions.len() * 8 + m.indices.len()) as u64; // the values in the fingerprint
        self.report(&m.check());
    }

    fn report(&mut self, r: &Report) {
        for v in [r.vertices, r.triangles, r.points, r.edges, r.components, r.open_edges, r.nonmanifold_edges, r.flipped_edges, r.nonmanifold_points, r.inverted_components, r.degenerate, r.bad_normals] {
            self.u(v as u64);
        }
        self.u(r.euler as u64);
        self.f(r.area);
        self.f(r.volume);
        if let Some(b) = r.bounds {
            for v in [b.min, b.max] {
                self.f(v.x);
                self.f(v.y);
                self.f(v.z);
            }
        }
    }
}

/// the operations of the harness besides the examples: each primitive, profile operation and modifier
fn operations() -> Vec<Mesh> {
    let mut v = Vec::new();
    let star: Vec<[f64; 2]> = (0..11)
        .map(|i| {
            let (s, c) = geom::math::sin_cos_deg(360.0 * i as f64 / 11.0 + 7.0);
            let r = if i % 2 == 0 { 1.0 } else { 0.45 };
            [r * c, r * s]
        })
        .collect();
    v.push(geom::cuboid(1.25, 0.5, 2.0).unwrap());
    v.push(geom::cylinder(0.3, 1.7, 29).unwrap());
    v.push(geom::cone(0.8, 1.1, 17, 0.2).unwrap());
    v.push(geom::sphere(0.65, 23, 13).unwrap());
    v.push(geom::torus(1.3, 0.37, 31, 11).unwrap());
    v.push(geom::stairs(1.1, 2.3, 3.7, 13).unwrap());
    v.push(geom::arch(2.2, 3.1, 0.4, 0.3, 21).unwrap());
    v.push(geom::extrude(&star, 0.3).unwrap());
    v.push(geom::prism(&geom::poly::rounded_rect(3.0, 2.0, 0.4, 5), 2.5).unwrap());
    let helix: Vec<Vec3> = (0..=40)
        .map(|k| {
            let (s, c) = geom::math::sin_cos_deg(17.0 * k as f64);
            Vec3::new(1.5 * c, 0.07 * k as f64, 1.5 * s)
        })
        .collect();
    v.push(geom::sweep(&geom::poly::circle(0.12, 10), &helix, false, 35.0).unwrap());
    let loop3 = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.7, 0.0), Vec3::new(2.3, 0.0, 1.9), Vec3::new(0.2, 1.1, 2.2), Vec3::new(-0.5, 0.4, 1.0)];
    v.push(geom::sweep(&star, &loop3, true, 20.0).unwrap());
    let vase: Vec<[f64; 2]> = (0..=12)
        .map(|k| {
            let t = k as f64 / 12.0;
            [0.3 + 0.25 * geom::math::sin_cos_deg(300.0 * t).0, 1.5 * t]
        })
        .chain([[0.0, 1.5], [0.0, 0.0]])
        .collect();
    v.push(geom::lathe(&vase, 37, 30.0).unwrap());
    let base = geom::torus(1.0, 0.3, 19, 9).unwrap();
    let turn = geom::Affine::rotation_about(Vec3::new(0.3, 1.0, -0.6), 37.5).unwrap().then_after(&geom::Affine::translation(Vec3::new(0.1, -0.2, 0.3)));
    v.push(base.transformed(&turn));
    v.push(base.scaled(Vec3::new(-1.3, 0.7, 2.1)));
    v.push(base.mirrored(Axis::Z).flipped());
    v.push(base.array(3, Vec3::new(0.7, 0.1, -0.4)).unwrap());
    v.push(base.translated(Vec3::new(2.5, 0.0, 0.0)).radial(7, 360.0 / 7.0).unwrap());
    v.push(base.smooth(12.0));
    v.push(base.flat().uv_box(0.37));
    v.push(Mesh::merge(&[&base, &base.translated(Vec3::new(2.0, 0.0, 0.0))]).unwrap().weld(1e-9));
    // (G7g part 2) Boolean operations and bevels: a slab less a turned prism and a box flush with its face, turned
    // boxes joined, a sphere cut by a box, a solid turned and cut again, bevels of a box, a cylinder and a frustum
    let solid = |m: &Mesh| geom::Solid::from_convex(m).unwrap();
    let slab = geom::cuboid(3.0, 2.0, 0.3).unwrap();
    let prism = geom::cylinder(0.4, 1.0, 13).unwrap().rotated(Axis::X, 90.0).translated(Vec3::new(-0.6, 1.0, -0.5));
    let notch = geom::cuboid(0.8, 0.5, 0.3).unwrap().translated(Vec3::new(0.9, 1.5, 0.0));
    v.push(geom::difference(&slab, &[&prism, &notch]).unwrap());
    let turned: Vec<Mesh> = (0..4).map(|k| geom::cuboid(1.0, 0.7, 1.3).unwrap().rotated(Axis::Y, 23.0 * k as f64).translated(Vec3::new(0.45 * k as f64, 0.1 * k as f64, 0.0))).collect();
    v.push(geom::union(&turned.iter().collect::<Vec<_>>()).unwrap());
    v.push(geom::intersection(&[&geom::sphere(1.0, 19, 11).unwrap(), &geom::cuboid(1.5, 1.2, 1.5).unwrap().translated(Vec3::new(0.3, 0.2, -0.1))]).unwrap());
    let cut = solid(&slab).difference(&solid(&prism)).unwrap();
    let turn = geom::Affine::rotation_about(Vec3::new(0.2, 1.0, -0.3), 31.0).unwrap();
    v.push(cut.transformed(&turn).difference(&solid(&geom::cuboid(0.5, 3.0, 0.5).unwrap().translated(Vec3::new(0.7, -0.5, 0.0)))).unwrap().to_mesh(30.0).unwrap());
    v.push(geom::bevel(&geom::cuboid(1.2, 0.8, 0.6).unwrap(), 0.1, 3, None).unwrap());
    v.push(geom::bevel(&geom::cylinder(0.5, 1.0, 24).unwrap(), 0.08, 2, None).unwrap());
    v.push(geom::bevel(&geom::cone(0.7, 1.2, 12, 0.3).unwrap(), 0.05, 1, None).unwrap());
    v.push(geom::bevel(&geom::cuboid(1.0, 2.0, 0.5).unwrap(), 0.15, 4, Some(Axis::Y)).unwrap());
    v
}

/// -> (the checksum, the number of values in it)
pub fn run() -> (u64, u64) {
    let mut h = Hash { h: Fnv::new(), n: 0 };
    for ex in examples::ALL {
        let mut loader = examples::loader;
        let r = run::run_script(&format!("{}.jfs", ex.name), ex.source, &[], &mut loader);
        h.u(r.problems.len() as u64);
        h.u(r.fuel);
        for p in &r.model.parts {
            h.mesh(&p.mesh);
            h.u(p.material as u64);
        }
        let a = run::analyse(&r.model);
        h.report(&a.whole);
        h.u(a.fingerprint);
    }
    for m in operations() {
        h.mesh(&m);
    }
    (h.h.0, h.n)
}
