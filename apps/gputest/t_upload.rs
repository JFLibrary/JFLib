// t_upload -- the staging ring and what the UI's GPU output needs of lib/gpu (main.rs lists the
// checks): a partial upload costs about what its rectangles cost, a frame never waits

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
#[cfg(windows)]
use plat::{PW_DEFAULT, PW_KEEP, PW_NOOWNER, PWF_RESIZABLE, PWF_SYSFRAME, PWS_SHOWNA};

use crate::t_compute::readback;
#[cfg(windows)]
use crate::t_window::pump;
use crate::{Gpu, T, out, setup};

const MIB: u64 = 1 << 20;

/// the texture's texels through a read-back buffer
fn read_tex(t: &mut T, dev: &Device, tex: &Texture, layout: Layout) -> Option<Vec<u8>> {
    let n = (tex.width() * tex.height() * 4) as u64;
    let rb = readback(t, dev, n)?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.texture_barrier(tex, layout, Layout::CopySrc);
    e.copy_texture_to_buffer(TextureRegion::mip(tex, 0), &rb, DataLayout::default());
    e.texture_barrier(tex, Layout::CopySrc, layout);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(dev.queue().submit([cmd]), "submit")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let mut px = vec![0u8; n as usize];
    t.res(rb.read(0, &mut px), "read")?;
    Some(px)
}

/// a frame of w x h pixels, stride bytes per row: value(x, y, frame) in every pixel
fn frame(w: u32, h: u32, stride: u32, k: u32) -> Vec<u8> {
    let mut v = vec![0xEEu8; (stride * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = (y * stride + x * 4) as usize;
            v[i..i + 4].copy_from_slice(&(x.wrapping_mul(7) ^ y.wrapping_mul(31) ^ k.wrapping_mul(0x0101_0101)).to_le_bytes());
        }
    }
    v
}

pub fn upload(t: &mut T) {
    let Some(g) = setup(t) else { return };
    rects(t, &g);
    ring(t, &g);
    buffers(t, &g);
    #[cfg(windows)]
    if crate::windows(&g.dev) {
        swap(t, &g);
        return;
    }
    out("SKIP: no swapchains on this device");
}

fn rects(t: &mut T, g: &Gpu) -> Option<()> {
    let dev = &g.dev;
    let (w, h) = (300u32, 200u32);
    let stride = w * 4 + 64;
    let tex = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::COPY_SRC | TextureUsage::SAMPLED)),
                    "create_texture")?;
    let q = dev.queue();
    let a = frame(w, h, stride, 1);
    t.res(q.write_texture_rects(&tex, &[[0, 0, w as i32, h as i32]], &a, stride, Layout::Undefined, Layout::Sampled), "the whole frame")?;
    let b = frame(w, h, stride, 2);
    // two rectangles, one partly outside, one empty, one fully outside
    let rs = [[10, 20, 30, 40], [280, 190, 100, 100], [50, 50, 0, 10], [-40, -40, 20, 20], [-10, 100, 30, 5], [290, 5, 10, 0]];
    t.res(q.write_texture_rects(&tex, &rs, &b, stride, Layout::Sampled, Layout::Sampled), "rectangles")?;
    let px = read_tex(t, dev, &tex, Layout::Sampled)?;
    let inside = |r: &[i32; 4], x: i32, y: i32| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3];
    let mut bad = 0;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let src = if rs.iter().any(|r| inside(r, x, y)) { &b } else { &a };
            let want = &src[(y as u32 * stride + x as u32 * 4) as usize..][..4];
            let got = &px[((y as u32 * w + x as u32) * 4) as usize..][..4];
            if want != got {
                bad += 1;
            }
        }
    }
    t.ok(bad == 0, &format!("write_texture_rects: rectangles in place, clipped, with a padded stride ({bad} texels differ)"));
    let e1 = q.write_texture_rects(&tex, &[[0, 0, 5, 5]], &a[..100], stride, Layout::Sampled, Layout::Sampled);
    t.ok(matches!(e1, Err(GpuError::Invalid(_))), "write_texture_rects: data smaller than the texture: Invalid");
    let e2 = q.write_texture_rects(&tex, &[[0, 0, 5, 5]], &a, 4, Layout::Sampled, Layout::Sampled);
    t.ok(matches!(e2, Err(GpuError::Invalid(_))), "write_texture_rects: stride smaller than a row: Invalid");
    // from a larger image: a texture that holds a part of it
    let (sw, sh, ox, oy) = (40u32, 30u32, 100i32, 50i32);
    let small = t.res(dev.create_texture(&TextureDesc::d2(sw, sh, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::COPY_SRC | TextureUsage::SAMPLED)),
                      "create_texture")?;
    t.res(q.write_texture_rects_at(&small, &[[ox, oy, sw as i32, sh as i32]], (ox, oy), &a, stride, Layout::Undefined, Layout::Sampled), "the whole part")?;
    // inside, across the texture's corner, across the image's edge, outside the texture
    let rs = [[110, 60, 20, 10], [90, 40, 15, 15], [135, 75, 400, 400], [0, 0, 30, 30]];
    t.res(q.write_texture_rects_at(&small, &rs, (ox, oy), &b, stride, Layout::Sampled, Layout::Sampled), "the rectangles")?;
    let px = read_tex(t, dev, &small, Layout::Sampled)?;
    let mut bad = 0;
    for y in 0..sh as i32 {
        for x in 0..sw as i32 {
            let (ix, iy) = (x + ox, y + oy);
            let src = if rs.iter().any(|r| inside(r, ix, iy)) { &b } else { &a };
            let want = &src[(iy as u32 * stride + ix as u32 * 4) as usize..][..4];
            let got = &px[((y as u32 * sw + x as u32) * 4) as usize..][..4];
            if want != got {
                bad += 1;
            }
        }
    }
    t.ok(bad == 0, &format!("write_texture_rects_at: rectangles of a larger image at the origin, clipped ({bad} texels differ)"));
    let e3 = q.write_texture_rects_at(&small, &[[0, 0, 5, 5]], (0, 0), &a, 0, Layout::Sampled, Layout::Sampled);
    t.ok(matches!(e3, Err(GpuError::Invalid(_))), "write_texture_rects_at: stride 0: Invalid");
    // a rectangle reaching past the image's rows is cut to them: the bytes after them are not read
    let mut one = alloc::vec![0u8; stride as usize * 2];
    one[..4].copy_from_slice(&[9, 8, 7, 255]);
    t.res(q.write_texture_rects_at(&small, &[[0, 0, 5, 100]], (0, 0), &one, stride, Layout::Sampled, Layout::Sampled), "a short image")?;
    // `small` goes now, its copy still in the open upload buffer: it stays until that has run
    // (validate.uis: destroyed at once, the buffer would be invalid)
    Some(())
}

fn ring(t: &mut T, g: &Gpu) -> Option<()> {
    // a device of its own: its memory counts only this
    let dev = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "create_device")?;
    let (w, h) = (1920u32, 1080u32);
    let tex = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::COPY_SRC | TextureUsage::SAMPLED)),
                    "create_texture")?;
    let q = dev.queue();
    let f = frame(w, h, w * 4, 3);
    t.res(q.write_texture_rects(&tex, &[[0, 0, 4, 4]], &f, w * 4, Layout::Undefined, Layout::Sampled), "first upload")?;
    let id = t.res(q.flush(), "flush")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let base = dev.memory_reserved();
    out(&format!("  a 1080p texture and the first small upload: {:.1} MiB reserved", base as f64 / MIB as f64));
    t.ok(base < 8294400 + 4 * MIB, "the texture and little more (blocks grow, no 64 + 16 MiB at once)");
    // a frame of small rectangles again and again: what the UI does
    let f2 = frame(w, h, w * 4, 4);
    let mut peak = 0;
    // three frames in flight, as with a swapchain: without that a CPU device falls hundreds of
    // submissions behind, and what they read cannot go back into the ring
    let mut flight = [None; 3];
    let t0 = plat::time_us();
    for k in 0..2000 {
        let r = [[(k * 37) % 1800, (k * 53) % 1000, 64, 32], [(k * 11) % 1800, (k * 7) % 1000, 200, 20]];
        t.res(q.write_texture_rects(&tex, &r, &f2, w * 4, Layout::Sampled, Layout::Sampled), "small upload")?;
        let id = t.res(q.flush(), "flush")?;
        peak = peak.max(dev.memory_reserved());
        if let Some(prev) = flight[k as usize % 3].replace(id) {
            t.res(dev.wait(prev, 10_000_000_000), "wait")?;
        }
    }
    let us = (plat::time_us() - t0) as f64 / 2000.0;
    t.res(dev.wait_idle(), "wait_idle")?;
    out(&format!("  2000 small uploads: {us:.1} us each, at most {:.2} MiB more", (peak - base) as f64 / MIB as f64));
    t.ok(peak <= base + 2 * MIB, "small uploads stay in the ring (at most 2 MiB)");
    // a whole frame: a large buffer, gone a second later
    t.res(q.write_texture_rects(&tex, &[[0, 0, w as i32, h as i32]], &f, w * 4, Layout::Sampled, Layout::Sampled), "whole frame")?;
    t.res(q.flush(), "flush")?;
    let big = dev.memory_reserved();
    t.ok(big >= base + 7 * MIB, &format!("a whole frame takes a buffer of its own: {:.1} MiB", (big - base) as f64 / MIB as f64));
    t.res(dev.wait_idle(), "wait_idle")?;
    let t1 = plat::time_us();
    for _ in 0..5 {
        t.res(q.write_texture_rects(&tex, &[[0, 0, w as i32, h as i32]], &f, w * 4, Layout::Sampled, Layout::Sampled), "whole frame")?;
        let id = t.res(q.flush(), "flush")?;
        t.res(dev.wait(id, 10_000_000_000), "wait")?;
    }
    out(&format!("  a whole frame again: {:.2} ms each", (plat::time_us() - t1) as f64 / 5000.0));
    t.ok(dev.memory_reserved() <= big, "whole frames reuse that buffer");
    let px = read_tex(t, &dev, &tex, Layout::Sampled)?;
    let same = px.chunks_exact((w * 4) as usize).zip(f.chunks_exact((w * 4) as usize)).all(|(a, b)| a == b);
    t.ok(same, "a whole frame arrives byte for byte");
    // (not sleep_ms: on Android, as in the browser, the main thread does not sleep)
    let t2 = plat::time_us();
    while plat::time_us() - t2 < 1_100_000 {
        plat::thread_yield();
    }
    t.res(dev.poll(), "poll")?;
    let after = dev.memory_reserved();
    t.ok(after <= base + 2 * MIB, &format!("a second later the large buffer is gone: {:.1} MiB reserved", after as f64 / MIB as f64));
    Some(())
}

fn buffers(t: &mut T, g: &Gpu) -> Option<()> {
    let dev = &g.dev;
    let q = dev.queue();
    // across the chunks of the ring, with submissions in between
    let n = 4000usize;
    let buf = t.res(dev.create_buffer(&BufferDesc { size: (n * 1024) as u64, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), "create_buffer")?;
    let mut block = [0u32; 256];
    for k in 0..n {
        for (i, v) in block.iter_mut().enumerate() {
            *v = (k * 1000 + i) as u32;
        }
        t.res(q.write_buffer(&buf, (k * 1024) as u64, &block), "write_buffer")?;
        if k % 97 == 0 {
            t.res(q.flush(), "flush")?;
        }
    }
    let mut out_ = vec![0u32; n * 256];
    t.res(q.read_buffer(&buf, 0, &mut out_), "read_buffer")?;
    let bad = out_.iter().enumerate().filter(|(i, v)| **v != ((i / 256) * 1000 + i % 256) as u32).count();
    t.ok(bad == 0, &format!("{n} writes of 1 KiB through the ring: {bad} words differ"));
    Some(())
}

// on a child window (Windows; G4b: the UI's output on Linux has its own way)
#[cfg(windows)]
fn swap(t: &mut T, g: &Gpu) -> Option<()> {
    let dev = &g.dev;
    let w = plat::win_create("gputest", 320, 240, PWF_SYSFRAME | PWF_RESIZABLE, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT);
    if !t.ok(w >= 0, "plat window") {
        return None;
    }
    plat::win_show(w, PWS_SHOWNA);
    pump(100);
    let r = swap_on(t, g, dev, w);
    plat::win_destroy(w);
    pump(30);
    r
}

#[cfg(windows)]
fn clear(dev: &Device, sc: &mut Swapchain, fr: Frame) -> gpu::Result<SubmitId> {
    let sc = &*sc;
    let mut e = dev.encoder()?;
    e.begin_render(&RenderPassDesc {
        color: &[ColorAttachment { view: sc.view(&fr), load: Load::Clear, clear: [0.2, 0.3, 0.4, 1.0], store: true, before: Layout::Undefined, after: Layout::Present }],
        depth: None,
    });
    e.end_render();
    let cmd = e.finish()?;
    sc.present(fr, [cmd])
}

#[cfg(windows)]
fn swap_on(t: &mut T, g: &Gpu, dev: &Device, w: i32) -> Option<()> {
    // on a child window, as the UI's output
    let nw = plat::win_gpu_child(w);
    let Some(nw) = nw else {
        t.fail("the child window");
        return None;
    };
    t.ok(plat::win_display_adapter(w).is_some() || cfg!(not(windows)), "the adapter of the window's display");
    let surf = t.res(Surface::native(&g.inst, nw, w), "Surface::native")?;
    t.ok(g.adapter.can_present(&surf), "the adapter can present to it");
    let mut sc = t.res(dev.create_swapchain(&surf, &SwapchainDesc::DEFAULT), "create_swapchain")?;
    plat::win_gpu_child_show(w, true);
    // a burst: try_acquire never waits for the display
    let (mut presented, mut notready) = (0, 0);
    let mut calls: Vec<u64> = Vec::new();
    let t0 = plat::time_us();
    while plat::time_us() < t0 + 300_000 {
        pump(0);
        let t1 = plat::time_us();
        let a = t.res(sc.try_acquire(), "try_acquire")?;
        calls.push(plat::time_us() - t1);
        match a {
            TryAcquire::Frame(fr) => {
                t.res(clear(dev, &mut sc, fr), "present")?;
                presented += 1;
            }
            TryAcquire::NotReady => {
                notready += 1;
                plat::thread_yield();
            }
            TryAcquire::Stale => {
                t.fail("try_acquire: stale without a reason");
                return None;
            }
            TryAcquire::NoArea => {
                t.fail("try_acquire: no area");
                return None;
            }
        }
    }
    calls.sort_unstable();
    let p99 = calls[calls.len() * 99 / 100];
    out(&format!("  FIFO burst of 300 ms: {presented} frames, {notready} times no image, try_acquire p99 {:.2} ms, longest {:.2} ms", p99 as f64 / 1000.0, calls[calls.len() - 1] as f64 / 1000.0));
    t.ok(presented > 0 && notready > 0, "try_acquire: frames, and NotReady instead of waiting for the display");
    // (the longest call says what the scheduler did on a busy machine)
    t.ok(p99 < 3_000, "try_acquire does not wait for the display");
    // the window's size: Stale, until rebuild
    plat::win_set_rect(w, PW_KEEP, PW_KEEP, 400, 300);
    pump(100);
    let a = t.res(sc.try_acquire(), "try_acquire after the resize")?;
    t.ok(matches!(a, TryAcquire::Stale), "a new size: Stale, no rebuild on the way");
    plat::win_gpu_child(w);
    let t1 = plat::time_us();
    t.res(sc.rebuild(), "rebuild")?;
    out(&format!("  rebuild: {:.2} ms", (plat::time_us() - t1) as f64 / 1000.0));
    t.ok(sc.size() == (400, 300), "rebuild: the new size");
    let mut got = false;
    for _ in 0..200 {
        match t.res(sc.try_acquire(), "try_acquire")? {
            TryAcquire::Frame(fr) => {
                t.res(clear(dev, &mut sc, fr), "present")?;
                got = true;
                break;
            }
            _ => plat::sleep_ms(1),
        }
    }
    t.ok(got, "a frame after the rebuild");
    // the swapchain and its surface go with an upload still open: the swapchain first, at once;
    // the buffer stays until the upload has run (validate.uis)
    let buf = t.res(dev.create_buffer(&BufferDesc { size: 16, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), "a buffer")?;
    t.res(dev.queue().write_buffer(&buf, 0, &[0u32; 4]), "an upload left open")?;
    drop(sc);
    drop(surf);
    drop(buf);
    t.res(dev.queue().flush(), "the upload")?;
    plat::win_gpu_child_show(w, false);
    pump(10);
    Some(())
}
