// t_render -- the group "render": the client's scene (scene.rs) offscreen from a fixed camera, a sanity check of the
// picture (no golden image): the sky where the camera looks over the wall, the yard below it, the same frame twice
// gives the same pixels, figures change the picture where they stand, the weapon in front of the camera changes its
// lower right, an impact's particles show.  Without Vulkan or a usable GPU: SKIP.  Also the offscreen target the
// measurement (bench.rs) draws into.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use game::Vec3;
use gpu::{Access, Buffer, BufferDesc, BufferUsage, ColorAttachment, DataLayout, Device, DeviceDesc, GpuError, Layout, Load, MemoryKind, Texture, TextureDesc, TextureRegion, TextureUsage};

use crate::look::Pose;
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

pub fn run(c: &mut Ctx) {
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
}
