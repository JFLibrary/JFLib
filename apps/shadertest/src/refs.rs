// refs -- the benchmark's records and CPU references on sample data: hash, floor, sincos,
// move_ref, the cube, the camera and cull_ref against an independent clip-space test

use alloc::format;
use alloc::vec::Vec;

use shader::bench::{self, Camera, CubeVertex, DrawCmd, Object, Push};

use crate::T;

/// sin and cos of 2 pi u in f64 (reduction to [-pi, pi], Taylor series)
pub fn sincos64(u: f64) -> (f64, f64) {
    let r = u - (u + 0.5).floor_soft();
    let x = r * core::f64::consts::TAU;
    let (x2, mut s, mut c, mut ts, mut tc) = (x * x, 0.0, 0.0, x, 1.0);
    for n in 0..30 {
        let n = n as f64;
        s += ts;
        c += tc;
        ts *= -x2 / ((2.0 * n + 2.0) * (2.0 * n + 3.0));
        tc *= -x2 / ((2.0 * n + 1.0) * (2.0 * n + 2.0));
    }
    (s, c)
}

trait FloorSoft {
    fn floor_soft(self) -> f64;
}

impl FloorSoft for f64 {
    fn floor_soft(self) -> f64 {
        let t = self as i64 as f64;
        if t > self { t - 1.0 } else { t }
    }
}

pub fn sample_push(count: u32) -> Push {
    Push { time: 12.5, count, seed: 7, extent: bench::extent_for(count), object_index: 0 }
}

pub fn sample_camera(extent: f32) -> Camera {
    Camera::look_at([0.0, extent * 0.3, extent * 1.2], [0.0, 0.0, 0.0], 1.0, 16.0 / 9.0, 0.1, extent * 4.0)
}

/// clip = view_proj * (p, 1) in f64
fn clip(cam: &Camera, p: [f32; 3]) -> [f64; 4] {
    let m = |c: usize, r: usize| cam.view_proj[c * 4 + r] as f64;
    let mut o = [0.0; 4];
    for (r, x) in o.iter_mut().enumerate() {
        *x = m(0, r) * p[0] as f64 + m(1, r) * p[1] as f64 + m(2, r) * p[2] as f64 + m(3, r);
    }
    o
}

pub fn run(t: &mut T) {
    // hash and unit
    for (x, h) in [(0u32, 129_708_002u32), (1, 2_831_084_092), (2, 2_055_130_248), (12345, 4_099_845_390),
                   (u32::MAX, 3_861_530_882)] {
        t.eq(&format!("hash({})", x), bench::hash(x), h);
    }
    t.eq("unit(0)", bench::unit(0), 0.0);
    t.eq("unit(max)", bench::unit(u32::MAX), 16_777_215.0 / 16_777_216.0);
    // floor
    for (x, f) in [(1.5f32, 1.0f32), (-1.5, -2.0), (-0.3, -1.0), (2.0, 2.0), (-2.0, -2.0), (1e10, 1e10),
                   (8_388_607.5, 8_388_607.0), (-8_388_607.5, -8_388_608.0), (0.999_999_94, 0.0)] {
        t.eq(&format!("floor({})", x), bench::floor(x), f);
    }
    t.eq("floor(-0) keeps the sign", bench::floor(-0.0).to_bits(), (-0.0f32).to_bits());
    // sincos against f64
    let mut worst = 0.0f64;
    for k in -4000..4000 {
        let u = k as f32 * 0.000_731;
        let (s, c) = bench::sincos_turns(u);
        let (s6, c6) = sincos64(u as f64);
        worst = worst.max((s as f64 - s6).abs()).max((c as f64 - c6).abs());
    }
    t.check(&format!("sincos_turns within 1e-6 (worst {:e})", worst), worst < 1e-6);
    let (s, c) = bench::sincos_turns(0.25);
    t.check("sincos(1/4) = (1, 0)", (s - 1.0).abs() < 1e-6 && c.abs() < 1e-6);
    // move_ref
    let p = sample_push(10_000);
    let objs = bench::move_all(&p);
    t.eq("move_all count", objs.len(), 10_000);
    t.eq("move_ref deterministic", bench::move_ref(4711, &p), objs[4711]);
    let mut ok = [true; 6];
    for o in &objs {
        let q = o.rot;
        let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
        ok[0] &= (n2 - 1.0).abs() < 2e-5;
        ok[1] &= o.radius == o.scale * 1.732_050_8;
        ok[2] &= (0.25..0.75).contains(&o.scale);
        ok[3] &= o.color.iter().all(|c| (0.35..=1.0).contains(c));
        let r = ((o.pos[0] * o.pos[0] + o.pos[2] * o.pos[2]) as f64).sqrt_soft();
        ok[4] &= r >= p.extent as f64 * 0.1 - 1e-3 && r <= p.extent as f64 + 1e-3;
        ok[5] &= o.pos[1].abs() <= p.extent * 0.125;
    }
    for (k, name) in ["unit quaternions", "radius = scale * sqrt 3", "scale range", "colour range", "orbit radius",
                      "height"].iter().enumerate() {
        t.check(&format!("move_ref {}", name), ok[k]);
    }
    let later = bench::move_ref(4711, &Push { time: 13.5, ..p });
    t.check("objects move over time", later.pos != objs[4711].pos && later.rot != objs[4711].rot);
    t.eq("colour and scale stay", (later.color, later.scale), (objs[4711].color, objs[4711].scale));
    t.check("the seed changes the scene", bench::move_ref(4711, &Push { seed: 8, ..p }) != objs[4711]);
    // records
    let o = objs[17];
    t.eq("Object bytes", o.bytes().len(), 48);
    t.eq("Object round trip", Object::from_bytes(&o.bytes()), o);
    t.eq("objects round trip", bench::objects_from_bytes(&bench::objects_bytes(&objs[..5])), objs[..5].to_vec());
    let d = DrawCmd { index_count: 36, instance_count: 77, first_index: 1, vertex_offset: -3, first_instance: 9 };
    t.eq("DrawCmd round trip", DrawCmd::from_bytes(&d.bytes()), d);
    t.eq("DrawCmd bytes", d.bytes().len(), 20);
    t.eq("DrawCmd::reset", DrawCmd::reset(), DrawCmd { index_count: 36, ..DrawCmd::default() });
    t.eq("Push bytes", p.bytes().len(), 20);
    t.eq("Push bytes: seed at 8", p.bytes()[8..12].to_vec(), 7u32.to_le_bytes().to_vec());
    t.eq("extent for 1 000 000", (bench::extent_for(1_000_000) + 0.5) as u32, 300);
    // the cube
    let vs: Vec<CubeVertex> = bench::cube_vertices();
    let is = bench::cube_indices();
    t.eq("cube vertices", vs.len(), 24);
    t.eq("cube indices", is.len(), 36);
    t.eq("cube bytes", bench::cube_bytes().len(), 24 * 32);
    let mut ccw = true;
    for tri in is.chunks(3) {
        let [a, b, c] = [vs[tri[0] as usize].pos, vs[tri[1] as usize].pos, vs[tri[2] as usize].pos];
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let fn_ = vs[tri[0] as usize].normal;
        ccw &= n[0] * fn_[0] + n[1] * fn_[1] + n[2] * fn_[2] > 0.0;
        ccw &= tri.iter().all(|&i| vs[i as usize].normal == fn_);
    }
    t.check("cube triangles counter-clockwise from outside", ccw);
    t.check("cube corners at +-1", vs.iter().all(|v| v.pos[..3].iter().all(|c| c.abs() == 1.0) && v.pos[3] == 1.0));
    // the camera
    let cam = Camera::look_at([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], 1.047_197_6, 16.0 / 9.0, 0.1, 1000.0);
    let o = clip(&cam, [0.0, 0.0, 0.0]);
    t.check("origin in the centre", o[0].abs() < 1e-5 && o[1].abs() < 1e-5 && (o[3] - 10.0).abs() < 1e-4);
    t.check("depth in 0..1", o[2] / o[3] > 0.0 && o[2] / o[3] < 1.0);
    let up = clip(&cam, [0.0, 1.0, 0.0]);
    t.check("up is up on screen (NDC y up, lib/gpu flips the viewport)", up[1] / up[3] > 0.0);
    let right = clip(&cam, [1.0, 0.0, 0.0]);
    t.check("right is right on screen", right[0] / right[3] > 0.0 && (right[1] / right[3]).abs() < 1e-6);
    // winding in NDC: the cube face towards the camera (+z) counter-clockwise, the one away from
    // it (-z) clockwise -- front faces with front_cw false, culled as back faces otherwise
    let area = |face: usize| {
        let p: Vec<[f64; 2]> = is[face * 6..face * 6 + 3].iter().map(|&i| {
            let v = vs[i as usize].pos;
            let c = clip(&cam, [v[0], v[1], v[2]]);
            [c[0] / c[3], c[1] / c[3]]
        }).collect();
        (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1])
    };
    let (front, back) = (vs.iter().position(|v| v.normal[2] == 1.0), vs.iter().position(|v| v.normal[2] == -1.0));
    let (front, back) = (front.unwrap_or(0) / 4, back.unwrap_or(0) / 4);
    t.check("face towards the camera counter-clockwise in NDC", area(front) > 0.0);
    t.check("face away from the camera clockwise in NDC", area(back) < 0.0);
    let near = clip(&cam, [0.0, 0.0, 9.9]);
    t.check("near plane at depth 0", (near[2] / near[3]).abs() < 1e-4);
    t.check("planes have unit normals", cam.planes.iter().all(|p| ((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]) - 1.0).abs() < 1e-5));
    t.eq("Camera bytes", cam.bytes().len(), 176);
    let sph = |pos: [f32; 3], radius: f32| Object { pos, radius, ..Object::default() };
    t.check("sphere at the origin visible", bench::visible(&sph([0.0, 0.0, 0.0], 1.0), &cam));
    t.check("behind the camera invisible", !bench::visible(&sph([0.0, 0.0, 20.0], 1.0), &cam));
    t.check("beyond far invisible", !bench::visible(&sph([0.0, 0.0, -2000.0], 1.0), &cam));
    t.check("far left invisible", !bench::visible(&sph([-1000.0, 0.0, 0.0], 1.0), &cam));
    // a sphere whose centre is just outside the left plane but which reaches into the frustum
    let lp = cam.planes[0];
    let c = [-lp[0] * (lp[3] + 0.5), -lp[1] * (lp[3] + 0.5), -lp[2] * (lp[3] + 0.5)];
    t.check("straddling sphere visible", bench::visible(&sph(c, 1.0), &cam));
    t.check("same centre, small radius invisible", !bench::visible(&sph(c, 0.25), &cam));
    // visible() for points against the clip-space test, away from the boundaries
    let mut agree = true;
    let mut seen = 0;
    for k in 0..20_000u32 {
        let h = bench::hash(k);
        let p = [(bench::unit(h) - 0.5) * 60.0, (bench::unit(bench::hash(h)) - 0.5) * 40.0,
                 (bench::unit(bench::hash(h ^ 0x55)) - 0.5) * 60.0];
        let c = clip(&cam, p);
        let m = [c[3] + c[0], c[3] - c[0], c[3] + c[1], c[3] - c[1], c[2], c[3] - c[2]];
        if m.iter().any(|x| x.abs() < 1e-3) {
            continue;
        }
        let inside = m.iter().all(|&x| x > 0.0);
        seen += inside as u32;
        agree &= inside == bench::visible(&sph(p, 0.0), &cam);
    }
    t.check(&format!("visible() = clip-space test ({} of 20000 inside)", seen), agree && seen > 100);
    // cull_ref on the moved scene
    let cam = sample_camera(p.extent);
    let list = bench::cull_ref(&objs, &cam);
    t.check(&format!("cull_ref keeps part of the scene ({} of {})", list.len(), objs.len()), !list.is_empty() && list.len() < objs.len());
    t.check("cull_ref ascending", list.windows(2).all(|w| w[0] < w[1]));
}

trait SqrtSoft {
    fn sqrt_soft(self) -> f64;
}

impl SqrtSoft for f64 {
    fn sqrt_soft(self) -> f64 {
        if self <= 0.0 {
            return 0.0;
        }
        let mut r = if self > 1.0 { self } else { 1.0 };
        for _ in 0..60 {
            r = 0.5 * (r + self / r);
        }
        r
    }
}
