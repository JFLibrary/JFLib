// t_render -- the group "render": the client's scene (scene.rs) offscreen from a fixed camera, a sanity check of the
// picture (no golden image): the sky where the camera looks over the wall, the yard below it, the same frame twice
// gives the same pixels, figures change the picture where they stand, the weapon in front of the camera changes its
// lower right quarter only and leaves the crosshair's surroundings free (16:9, 4:3, 21:9 and a narrow window), an
// impact's particles show.  Without Vulkan or a usable GPU: SKIP; before that, without a GPU, the weapon's place
// through its lens on windows from 9:21 to 32:9 in rest, kicking, lowered and bobbing, and the own tracer's start
// on the screen at the muzzle for fields of view of 70 to 120 degrees.  Also the offscreen target the measurement
// (bench.rs) draws into.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use game::Vec3;
use gpu::{Access, Buffer, BufferDesc, BufferUsage, ColorAttachment, DataLayout, Device, DeviceDesc, GpuError, Layout, Load, MemoryKind, Texture, TextureDesc, TextureRegion, TextureUsage};

use crate::look::{self, Pose, VmState};
use crate::scene::{Frame, Fx, View};
use crate::{out, Ctx};

/// an offscreen target (the view's format) and the read-back of its image
pub struct Off {
    pub w: u32,
    pub h: u32,
    stride: u32,
    color: Texture,
    rb: Buffer,
    used: bool,
}

impl Off {
    pub fn new(dev: &Device, w: u32, h: u32) -> gpu::Result<Off> {
        let stride = (w * 4).div_ceil(256) * 256;
        Ok(Off {
            w,
            h,
            stride,
            color: dev.create_texture(&TextureDesc::d2(w, h, ui::GPU_VIEW_FORMAT, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?,
            rb: dev.create_buffer(&BufferDesc { size: stride as u64 * h as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?,
            used: false,
        })
    }

    /// one frame of the view; `read`: copy it back and wait (rows without padding, B G R x), else only submit
    pub fn frame(&mut self, dev: &Device, v: &mut View, read: bool, wait: bool) -> gpu::Result<Option<Vec<u8>>> {
        v.prepare(self.w, self.h)?;
        let mut e = dev.encoder()?;
        let color = ColorAttachment { view: self.color.view(), load: Load::Clear, clear: [0.0; 4], store: true, before: if self.used { Layout::CopySrc } else { Layout::Undefined }, after: Layout::CopySrc };
        v.record(&mut e, color, None)?;
        if read {
            e.copy_texture_to_buffer(TextureRegion::mip(&self.color, 0), &self.rb, DataLayout { offset: 0, bytes_per_row: self.stride, rows_per_image: 0 });
            e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        }
        let cmd = e.finish()?;
        let id = dev.queue().submit([cmd])?;
        v.submitted_as(id);
        self.used = true;
        if !(read || wait) {
            return Ok(None);
        }
        if !dev.wait(id, 10_000_000_000)? {
            return Err(GpuError::Invalid("the GPU did not finish within 10 s"));
        }
        if !read {
            return Ok(None);
        }
        let mut raw = vec![0u8; self.rb.size() as usize];
        self.rb.read(0, &mut raw)?;
        let mut img = Vec::with_capacity((self.w * self.h * 4) as usize);
        for y in 0..self.h as usize {
            img.extend_from_slice(&raw[y * self.stride as usize..y * self.stride as usize + (self.w * 4) as usize]);
        }
        Ok(Some(img))
    }
}

/// a device for offscreen work, or why there is none (SKIP)
pub fn device(name: &str) -> Result<Device, alloc::string::String> {
    let inst = gpu::Instance::with(&gpu::InstanceDesc { app_name: "game1", validate: false }).map_err(|e| format!("{}", e))?;
    let adapter = inst.adapter().map_err(|e| format!("{}", e))?;
    let dev = adapter.create_device(&DeviceDesc::DEFAULT).map_err(|e| format!("{}", e))?;
    out(&format!("  {}: {}, level {:?}\n", name, adapter.info().name, dev.level()));
    Ok(dev)
}

/// the shapes of window the weapon is checked on (width / height)
const ASPECTS: [(&str, f64); 9] = [("16:9", 16.0 / 9.0), ("16:10", 1.6), ("4:3", 4.0 / 3.0), ("5:4", 1.25), ("21:9", 64.0 / 27.0), ("32:9", 32.0 / 9.0), ("1:1", 1.0), ("9:16", 9.0 / 16.0), ("9:21", 9.0 / 21.0)];

/// the square around the crosshair the weapon leaves free: this much of the window's measure (its height, on a
/// window narrower than 4:3 its width / (4/3)) to each side
const FREE: f64 = 0.15;

/// how far from the right edge the HUD's ammunition reaches (in heights: its margin and "17 / 20" at 34 / 900,
/// measured 0.172 at 1280 x 720 and 1680 x 720)
const AMMO: f64 = 0.175;

fn tan_half(fov: f64) -> f64 {
    let (s, c) = physics::math::sincos(fov * 0.5);
    s / c
}

/// the weapon in front of the camera without a GPU: every vertex of its parts through the viewmodel's lens on every
/// shape of window, in rest, kicking, lowered and bobbing: all of it right of and below the crosshair, the square
/// FREE around it empty, some of it on the screen; the own tracer starts where the screen shows the muzzle (the
/// world's camera with fields of view 70 to 120 degrees, horizontal at 16:9 as the settings count them)
fn layout(c: &mut Ctx, v: &View) {
    let t = tan_half(look::VM_FOV);
    let parts: Vec<&game1_core::map::PartData> = v.figure.parts.iter().filter(|p| look::in_viewmodel(&p.name)).collect();
    let verts: usize = parts.iter().map(|p| p.positions.len()).sum();
    c.check(parts.len() == 3 && verts > 0, || format!("the weapon's parts: {} with {} vertices", parts.len(), verts));
    let mut states = vec![("rest", VmState::default()), ("kick", VmState { kick: 1.0, ..VmState::default() }), ("lowered", VmState { reload: 1.0, ..VmState::default() })];
    for k in 0..8 {
        let bob = k as f64 * core::f64::consts::FRAC_PI_4;
        states.push(("bobbing", VmState { bob, bob_amount: 1.0, ..VmState::default() }));
        states.push(("bobbing, kick, half lowered", VmState { bob, bob_amount: 1.0, kick: 1.0, reload: 0.5 }));
    }
    let mut closest = f64::INFINITY;
    let mut gap = f64::INFINITY;
    for (name, a) in ASPECTS {
        let fit = look::vm_fit(a);
        for (k, (state, vm)) in states.iter().enumerate() {
            let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for p in &parts {
                for q in &p.positions {
                    let m = look::viewmodel_point(v.anchor, vm, a, Vec3::new(q[0] as f64, q[1] as f64, q[2] as f64));
                    let (x, y) = (m.x / (-m.z * t * a), m.y / (-m.z * t));
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
            // the box's corner next to the crosshair, in the window's measure (half a height is 1 in y)
            let right = x0 * a * 0.5 / fit;
            let below = -y1 * 0.5 / fit;
            closest = closest.min(right.max(below));
            c.check(x0 > 0.0 && y1 < 0.0 && right.max(below) >= FREE, || format!("{} {}: the weapon reaches x {:.3} y {:.3} (from the crosshair {:.3} right, {:.3} below; free {})", name, state, x0, y1, right, below, FREE));
            c.check(x0 < 0.9 && y1 > -0.9 && x1 > x0 && y1 > y0, || format!("{} {}: the weapon is hardly on the screen: x {:.3}..{:.3}, y {:.3}..{:.3}", name, state, x0, x1, y0, y1));
            // at rest on 4:3 to 21:9 it ends left of the ammunition (the HUD's lower right corner: AMMO)
            if k == 0 && fit >= 1.0 && a <= 2.4 {
                let g = (1.0 - x1) * a * 0.5;
                gap = gap.min(g);
                c.check(g >= AMMO, || format!("{} at rest: the weapon ends {:.3} heights from the right edge (the ammunition takes {})", name, g, AMMO));
            }
        }
    }
    // the own tracer's start
    let vm = VmState { kick: 1.0, bob: 0.7, bob_amount: 1.0, reload: 0.0 };
    let mut worst = 0.0f64;
    for (_, a) in ASPECTS {
        for deg in [70.0, 100.0, 120.0] {
            let fov = 2.0 * physics::math::atan2(tan_half(deg * core::f64::consts::PI / 180.0) / (16.0 / 9.0), 1.0);
            let (eye, yaw, pitch) = (Vec3::new(3.0, 1.65, -7.0), 0.9, -0.35);
            let m = look::viewmodel_point(v.anchor, &vm, a, v.muzzle);
            let w = look::vm_to_world(m, eye, yaw, pitch, fov);
            let mut fp = game::FirstPerson::new(eye);
            fp.yaw = yaw;
            fp.pitch = pitch;
            let view = fp.view();
            let right = view.forward.cross(view.up);
            let d = w - eye;
            let (x, y, z) = (d.dot(right), d.dot(view.up), d.dot(view.forward));
            let tw = tan_half(fov);
            let err = (x / (z * tw * a) - m.x / (-m.z * t * a)).abs().max((y / (z * tw) - m.y / (-m.z * t)).abs());
            worst = worst.max(err);
            c.check(err < 1e-9 && z > 0.05, || format!("aspect {:.3}, field of view {}: the tracer starts {:.2e} away from the muzzle on the screen (depth {:.3})", a, deg, err, z));
        }
    }
    c.note(&format!("the weapon: {} vertices, {} windows x {} states, the crosshair's surroundings free by {:.3} at least ({}), at rest {:.3} from the right edge ({}); the own tracer's start off the muzzle by {:.1e}", verts, ASPECTS.len(), states.len(), closest, FREE, gap, AMMO, worst));
}

fn diff(a: &[u8], b: &[u8], w: u32, region: [u32; 4]) -> u32 {
    let mut n = 0;
    for y in region[1]..region[3] {
        for x in region[0]..region[2] {
            let i = ((y * w + x) * 4) as usize;
            if a[i..i + 3] != b[i..i + 3] {
                n += 1;
            }
        }
    }
    n
}

/// the camera of the test: above the south wall, looking north over the yard
pub fn test_frame() -> Frame {
    Frame { eye: Vec3::new(0.0, 7.0, 24.0), yaw: 0.0, pitch: -0.28, fov: 1.0, ..Frame::default() }
}

/// figures in a row in front of the camera
pub fn test_figures(n: usize, t: f64) -> Vec<(u16, Pose)> {
    (0..n)
        .map(|k| {
            let x = -8.0 + 16.0 * k as f64 / (n.max(2) - 1) as f64;
            (k as u16 + 1, Pose { feet: Vec3::new(x, 0.0, 12.0 - (k % 3) as f64 * 3.0), yaw: t + k as f64, pitch: 0.1 * k as f64 - 0.3, crouched: k % 5 == 4, grounded: true, phase: t * 8.0 + k as f64, swing: 1.0 })
        })
        .collect()
}

/// the weapon in front of the camera on a w x h window: it changes some pixels of the lower right quarter, none
/// elsewhere, none in the square FREE around the crosshair
fn weapon_pixels(c: &mut Ctx, dev: &Device, v: &mut View, f: &Frame, (w, h): (u32, u32)) {
    let mut off = match Off::new(dev, w, h) {
        Ok(o) => o,
        Err(e) => {
            c.check(false, || format!("target {} x {}: {}", w, h, e));
            return;
        }
    };
    // (later than every particle's life: the two frames differ by the weapon alone)
    let mut pics = Vec::new();
    for show in [false, true] {
        v.frame = Frame { show_vm: show, time: f.time + 10.0, ..f.clone() };
        match off.frame(dev, v, true, true) {
            Ok(Some(img)) => pics.push(img),
            r => {
                c.check(false, || format!("frame {} x {}: {:?}", w, h, r.err()));
                return;
            }
        }
    }
    let r = (FREE * h.min(w * 3 / 4) as f64) as u32;
    let lower_right = diff(&pics[0], &pics[1], w, [w / 2, h / 2, w, h]);
    let rest = diff(&pics[0], &pics[1], w, [0, 0, w, h]) - lower_right;
    let centre = diff(&pics[0], &pics[1], w, [w / 2 - r, h / 2 - r, w / 2 + r, h / 2 + r]);
    c.check(lower_right > w * h / 200 && rest == 0 && centre == 0, || format!("{} x {}: the weapon changed {} pixels lower right, {} elsewhere, {} around the crosshair", w, h, lower_right, rest, centre));
    c.note(&format!("{} x {}: the weapon {} px, all lower right, none within {} px of the crosshair", w, h, lower_right, r));
}

pub fn run(c: &mut Ctx) {
    layout(c, &View::new(game1_core::map::werft()));
    let dev = match device("render") {
        Ok(d) => d,
        Err(e) => {
            out(&format!("  render: SKIP: {}\n", e));
            return;
        }
    };
    let (w, h) = (480u32, 270u32);
    let mut v = View::new(game1_core::map::werft());
    if let Err(e) = v.attach_to(&dev) {
        c.check(false, || format!("attach: {}", e));
        return;
    }
    let mut off = match Off::new(&dev, w, h) {
        Ok(o) => o,
        Err(e) => {
            c.check(false, || format!("target: {}", e));
            return;
        }
    };
    let mut shot = |v: &mut View, f: Frame, c: &mut Ctx| -> Vec<u8> {
        v.frame = f;
        match off.frame(&dev, v, true, true) {
            Ok(Some(img)) => img,
            r => {
                c.check(false, || format!("frame: {:?}", r.err()));
                vec![0; (w * h * 4) as usize]
            }
        }
    };
    let empty = shot(&mut v, test_frame(), c);
    let again = shot(&mut v, test_frame(), c);
    c.check(empty == again, || "the same frame twice gave other pixels".into());
    // the sky: the top row is the clear colour (linear light, encoded to sRGB codes by render3d: one colour, blue
    // above red)
    let want = [empty[0] as i32, empty[1] as i32, empty[2] as i32];
    c.check(want[0] > want[2] + 30, || format!("the sky's corner is not blue: B G R {:?}", want));
    let mut skyish = 0;
    for x in 0..w {
        let i = (x * 4) as usize;
        if (0..3).all(|k| (empty[i + k] as i32 - want[k]).abs() <= 2) {
            skyish += 1;
        }
    }
    c.check(skyish > w * 9 / 10, || format!("{} of {} pixels of the top row are sky ({:?} against {:?}, the middle row {:?})", skyish, w, &empty[..4], want, &empty[(w * h * 2) as usize..(w * h * 2) as usize + 4]));
    // the yard: the lower half is not sky and not one colour
    let mut colours = alloc::collections::BTreeSet::new();
    let mut not_sky = 0;
    for y in h / 2..h {
        for x in (0..w).step_by(7) {
            let i = ((y * w + x) * 4) as usize;
            colours.insert((empty[i] >> 3, empty[i + 1] >> 3, empty[i + 2] >> 3));
            if (0..3).any(|k| (empty[i + k] as i32 - want[k]).abs() > 8) {
                not_sky += 1;
            }
        }
    }
    c.check(colours.len() > 12 && not_sky > 0, || format!("the yard: {} colours, {} pixels not sky", colours.len(), not_sky));
    // figures change the middle of the picture
    let mut f = test_frame();
    f.figures = test_figures(6, 0.5);
    let with = shot(&mut v, f.clone(), c);
    let d = diff(&empty, &with, w, [0, h / 4, w, h]);
    c.check(d > 600, || format!("six figures changed {} pixels", d));
    // the weapon in front of the camera: the lower right
    let mut g = f.clone();
    g.show_vm = true;
    let vm = shot(&mut v, g.clone(), c);
    let dr = diff(&with, &vm, w, [w / 2, h / 2, w, h]);
    let dl = diff(&with, &vm, w, [0, 0, w / 3, h / 2]);
    c.check(dr > 500 && dl == 0, || format!("the weapon changed {} pixels lower right and {} upper left", dr, dl));
    // particles: an impact in the yard and the own muzzle flash
    v.fx.push(Fx::Impact { at: Vec3::new(0.0, 0.0, 15.0), normal: Vec3::new(0.0, 1.0, 0.0) });
    v.fx.push(Fx::MuzzleOwn);
    g.time += 0.05;
    let px = shot(&mut v, g.clone(), c);
    let dp = diff(&vm, &px, w, [0, 0, w, h]);
    c.check(dp > 20, || format!("the particles changed {} pixels", dp));
    c.note(&format!("{} x {}: figures {} px, weapon {} px, particles {} px, instances {}", w, h, d, dr, dp, v.stats.instances));
    // the weapon on other shapes of window: 16:9, 4:3, 21:9, narrow
    for size in [(480, 270), (360, 270), (630, 270), (150, 270)] {
        weapon_pixels(c, &dev, &mut v, &f, size);
    }
}
