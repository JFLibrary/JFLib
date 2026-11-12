// blocks -- render2dtest's tests of blocks, inserts and transparency (G6b2; main.rs lists them)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Cap, Join, Layer, Renderer, RendererDesc, Scene, Shape, Style, Transparency};

use crate::gallery::*;
use crate::refimg::{self, Tol};
use crate::tests::{dump, gpu_image, on_levels, shot, T};

/// a block drawing's image on the GPU (a new scene)
pub fn block_image(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, d: &BlockDrawing, cam: &Camera, what: &str) -> Option<Vec<u8>> {
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    t.res(s.set_camera(cam), what)?;
    t.res(build_blocks(&mut s, d), what)?;
    shot(t, dev, off, r, &mut s, what)
}

/// pixels further apart than `small` codes, and the largest difference
fn apart(a: &[u8], b: &[u8], small: u8) -> (u32, u8) {
    let (mut n, mut m) = (0u32, 0u8);
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let d = (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0);
        m = m.max(d);
        n += (d > small) as u32;
    }
    (n, m)
}

fn name_of(what: &str) -> String {
    what.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// the tolerance of a block drawing against the same drawing expanded and drawn as plain entities: lines and arcs where they
/// were (f64 against double-f32 through the transform: a code now and then where a pixel's owner is a tie), curves and
/// fills flattened in the block's units against in the drawing's (both within 1/32 px: up to 8 codes along their edges)
const TOL_EXPANDED: Tol = Tol { small: 2, over: 900, max: 48, ink: 0.004 };

pub fn blocks(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = blocks_on(t, dev, level); });
    block_errors(t);
}

fn blocks_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (360u32, 270u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    // the drawing of blocks against itself expanded (drawn as plain entities) and against lib/gfx
    for uneven in [false, true] {
        let bd = blocks_drawing(uneven);
        let what = format!("{level}: blocks inserted moved, turned, scaled, mirrored, as arrays, nested, styles by block{}", if uneven { ", scaled unevenly" } else { "" });
        let img = block_image(t, dev, &mut r, &mut off, &bd, &cam, &what)?;
        let ex = expand_blocks(&bd);
        let plain = gpu_image(t, dev, &mut r, &mut off, &ex, &cam, &what)?;
        let reference = refimg::draw(&ex, &cam, (w, h), BG);
        dump(&format!("{}_gpu", name_of(&what)), &img, w, h);
        dump(&format!("{}_plain", name_of(&what)), &plain, w, h);
        dump(&format!("{}_ref", name_of(&what)), &reference, w, h);
        let c = refimg::compare(&img, &plain, w, BG, TOL_EXPANDED.small);
        t.ok(c.fine(&TOL_EXPANDED), &format!("{what}: against the same drawing expanded ({} entities) as plain entities: {}", ex.items.len(), c.text()));
        // (against lib/gfx: as near as the same entities drawn plainly are, a few pixels more where curves are flattened
        // in the block's units; the drawing mixes what each test of strokes, arcs, dashes, curves and hatches allows)
        let cp = refimg::compare(&plain, &reference, w, BG, 12);
        let ink_p = (cp.ink[0] - cp.ink[1]).abs() / cp.ink[1].max(1.0);
        let tol = Tol { small: 12, over: cp.over + 64, max: cp.max.saturating_add(24), ink: ink_p + 0.002 };
        let c = refimg::compare(&img, &reference, w, BG, tol.small);
        t.ok(c.fine(&tol), &format!("{what}: against lib/gfx: {} (the expanded drawing: {})", c.text(), cp.text()));
    }
    // translucent: one insert (mirrored and turned) blends each pixel of its polyline's sharp corners, its arc's joints and
    // its fill once; two overlapping inserts blend each (as AutoCAD: per object), as the expanded drawing does
    for alpha in [0x80u32, 0x40] {
        let mut bd = BlockDrawing { layers: vec![(true, 0, 0)], blocks: Vec::new(), items: Vec::new() };
        let c = (alpha << 24) | 0x0020_40C0;
        let mut b = BDef { base: [0.0, 0.0], items: Vec::new() };
        b.items.push((style(c, 9.0, Cap::Butt, Join::Miter), BGeo::G(Geo::Poly(vec![[0.0, 0.0], [60.0, 8.0], [5.0, 30.0], [70.0, 50.0]], false))));
        b.items.push((style(c, 6.0, Cap::Round, Join::Round), BGeo::G(Geo::Arc([40.0, 70.0], 25.0, 0.2, 4.0))));
        b.items.push((style(c, 0.0, Cap::Butt, Join::Miter), BGeo::G(Geo::Fill(vec![vec![Geo::Circle([90.0, 30.0], 18.0)]]))));
        bd.blocks.push(b);
        let one = BlockDrawing { items: vec![(0, style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), BGeo::Ins(BIns::new(0, [200.0, 40.0]).turned(2.5, [1.3, -1.3])))], ..bd.clone() };
        let what = format!("{level}: a translucent ({alpha:#x}) block inserted mirrored and turned");
        let img = block_image(t, dev, &mut r, &mut off, &one, &cam, &what)?;
        dump(&format!("{}_gpu", name_of(&what)), &img, w, h);
        let twice = blended_twice(&img, alpha);
        t.ok(twice == 0, &format!("{what}: no pixel beyond one blend of the colour: {twice}"));
        bd.items.push((0, style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), BGeo::Ins(BIns::new(0, [40.0, 60.0]))));
        bd.items.push((0, style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), BGeo::Ins(BIns::new(0, [70.0, 80.0]).turned(0.4, [1.1, 1.1]))));
        let what = format!("{level}: two translucent ({alpha:#x}) inserts overlapping");
        let img = block_image(t, dev, &mut r, &mut off, &bd, &cam, &what)?;
        let plain = gpu_image(t, dev, &mut r, &mut off, &expand_blocks(&bd), &cam, &what)?;
        let (n, m) = apart(&img, &plain, 2);
        let darker = blended_twice(&img, alpha);
        t.ok(n <= 40 && m <= 4 && darker > 100, &format!("{what}: each blends (where they overlap {darker} pixels beyond one blend), as the expanded drawing: {n} pixels more than 2 codes apart, largest {m}"));
    }
    block_changes(t, dev, &mut r, &mut off, level)
}

/// the scene against one built anew (byte for byte): a block's change goes up once, moved inserts only as their
/// transforms
#[allow(clippy::too_many_arguments)]
fn judge_anew(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, s: &mut Scene, bd: &BlockDrawing, cam: &Camera, what: &str) -> Option<(render2d::UploadInfo, render2d::InstanceInfo)> {
    let img = shot(t, dev, off, r, s, what)?;
    let (info, ii) = (s.upload_info(), s.instance_info());
    let fresh = block_image(t, dev, r, off, bd, cam, what)?;
    let (n, _) = differ(&img, &fresh);
    t.ok(n == 0, &format!("{what}: against the scene built anew: {n} pixels differ ({} records sent, {} of blocks; {} instances, {} sent{})", info.records, ii.sent_block_records, ii.instances,
                          ii.sent_instances, if ii.rebuilt { ", laid out anew" } else { "" }));
    Some((info, ii))
}

fn block_changes(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, level: &str) -> Option<()> {
    let cam = pixel_camera(off.w, off.h);
    // a block of 40 lines inserted 1000 times (an array would do it in one: these are single inserts), a plain line
    let mut bd = BlockDrawing { layers: vec![(true, 0, 0), (true, 1, 0)], blocks: Vec::new(), items: Vec::new() };
    let mut b = BDef { base: [0.0, 0.0], items: Vec::new() };
    let mut rng = Rng(5);
    for k in 0..40 {
        let a = [rng.range(0.0, 8.0), rng.range(0.0, 8.0)];
        b.items.push((style(INK[k % 4], rng.range(0.0, 1.5) as f32, Cap::Butt, Join::Miter), BGeo::G(Geo::Line(a, [a[0] + rng.range(-4.0, 4.0), a[1] + rng.range(-4.0, 4.0)]))));
    }
    bd.blocks.push(b);
    bd.items.push((0, style(0xFF80_8080, 3.0, Cap::Round, Join::Round), BGeo::G(Geo::Line([10.0, 10.0], [350.0, 260.0]))));
    for k in 0..1000 {
        let at = [5.0 + (k % 40) as f64 * 8.8, 5.0 + (k / 40) as f64 * 10.4];
        bd.items.push((1, style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), BGeo::Ins(BIns::new(0, at).turned(0.01 * k as f64, [1.0, 1.0]))));
    }
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    t.res(s.set_camera(&cam), "camera")?;
    let built = t.res(build_blocks(&mut s, &bd), "build")?;
    judge_anew(t, dev, r, off, &mut s, &bd, &cam, &format!("{level}: a block of 40 lines inserted 1000 times"))?;
    let ii = s.instance_info();
    t.ok(ii.instances == 1000 && ii.block_records == 40 && ii.variants == 1, &format!("{level}: its records lie once: {} records of blocks, {} variants, {} instances", ii.block_records, ii.variants, ii.instances));
    // nothing changed, the camera moved: the view alone goes up
    let mut c2 = cam;
    c2.pan(3.0, 1.0);
    t.res(s.set_camera(&c2), "camera")?;
    let _ = shot(t, dev, off, r, &mut s, "panned")?;
    let (info, ii) = (s.upload_info(), s.instance_info());
    t.ok(info.bytes == 80 && !ii.rebuilt && ii.sent_instances == 0, &format!("{level}: a frame of a pan sends the view only: {} bytes, {} instances, laid out anew {}", info.bytes, ii.sent_instances, ii.rebuilt));
    t.res(s.set_camera(&cam), "camera")?;
    // the block changes: one line more, one moved; all inserts show it, its records go up once
    let sid = t.res(s.add_style(&style(0xFFD0_2020, 1.0, Cap::Round, Join::Round)), "add_style")?;
    t.res(s.insert_in(built.blocks[0], sid, &Shape::Line { a: [0.0, 0.0], b: [8.0, 8.0] }), "insert_in")?;
    bd.blocks[0].items.push((style(0xFFD0_2020, 1.0, Cap::Round, Join::Round), BGeo::G(Geo::Line([0.0, 0.0], [8.0, 8.0]))));
    t.res(s.update(built.block_items[0][3], &Shape::Line { a: [1.0, 1.0], b: [6.0, 2.0] }), "update")?;
    bd.blocks[0].items[3].1 = BGeo::G(Geo::Line([1.0, 1.0], [6.0, 2.0]));
    let (info, ii) = judge_anew(t, dev, r, off, &mut s, &bd, &cam, &format!("{level}: the block changed (a line more, one moved)"))?;
    t.ok(ii.sent_block_records == 41 && info.records == 41, &format!("{level}: the block's records go up once for its 1000 inserts: {} of blocks, {} in all", ii.sent_block_records, info.records));
    // 1000 inserts moved: their transforms alone
    for k in 0..1000 {
        if let (_, _, BGeo::Ins(b)) = &mut bd.items[1 + k] {
            b.at[0] += 1.25;
            b.rotation += 0.1;
            let ins = *b;
            let blocks = &built.blocks;
            t.res(with_bshape(&BGeo::Ins(ins), blocks, |sh| s.update(built.items[1 + k], sh)), "update")?;
        }
    }
    let (info, ii) = judge_anew(t, dev, r, off, &mut s, &bd, &cam, &format!("{level}: 1000 inserts moved"))?;
    t.ok(info.records == 0 && !info.rebuild && !ii.rebuilt && ii.sent_instances == 1000 && ii.sent_bytes == 160_000,
         &format!("{level}: 1000 inserts moved send their transforms only: {} instances, {} bytes, {} records", ii.sent_instances, ii.sent_bytes, info.records));
    // an insert removed, an insert's style (by block) changed, another style taken
    t.res(s.remove(built.items[500]), "remove")?;
    bd.items.remove(500);
    judge_anew(t, dev, r, off, &mut s, &bd, &cam, &format!("{level}: an insert removed"))?;
    Some(())
}

fn block_errors(t: &mut T) {
    let Some(g) = crate::tests::setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
    let bad = |r: gpu::Result<render2d::EntityId>| matches!(r, Err(GpuError::Invalid(_)));
    let Some(sid) = t.res(s.add_style(&Style::new(0xFF00_0000)), "add_style") else { return };
    let Some(l) = t.res(s.add_layer(&Layer::new(0)), "add_layer") else { return };
    let Some(a) = t.res(s.add_block(&render2d::Block::new()), "add_block") else { return };
    let Some(b) = t.res(s.add_block(&render2d::Block::new()), "add_block") else { return };
    let ins = |blk| Shape::Insert(render2d::Insert::new(blk, [0.0, 0.0]));
    t.ok(bad(s.insert_in(a, sid, &ins(a))), "a block inside itself is refused");
    let Some(_) = t.res(s.insert_in(a, sid, &ins(b)), "insert_in") else { return };
    t.ok(bad(s.insert_in(b, sid, &ins(a))), "a cycle of blocks is refused");
    // a chain of BLOCK_DEPTH levels: one more is refused
    let mut chain = vec![b];
    for _ in 0..render2d::BLOCK_DEPTH {
        let Some(n) = t.res(s.add_block(&render2d::Block::new()), "add_block") else { return };
        chain.push(n);
    }
    let mut refused_at = 0;
    for k in 1..chain.len() {
        if bad(s.insert_in(chain[k - 1], sid, &ins(chain[k]))) {
            refused_at = k;
            break;
        }
    }
    t.ok(refused_at as u32 == render2d::BLOCK_DEPTH - 1, &format!("more than BLOCK_DEPTH = {} levels of blocks are refused (refused at level {})", render2d::BLOCK_DEPTH, refused_at + 2));
    let mut i0 = render2d::Insert::new(a, [0.0, 0.0]);
    i0.scale = [0.0, 1.0];
    t.ok(bad(s.insert(l, sid, &Shape::Insert(i0))), "an insert of scale 0 is refused");
    let mut i1 = render2d::Insert::new(a, [0.0, 0.0]);
    i1.columns = 0;
    t.ok(bad(s.insert(l, sid, &Shape::Insert(i1))), "an insert of 0 columns is refused");
    let Some(e) = t.res(s.insert(l, sid, &ins(a)), "insert") else { return };
    t.ok(matches!(s.remove_block(a), Err(GpuError::Invalid(_))), "a block that is inserted cannot be removed");
    t.ok(s.remove(e).is_ok() && s.remove_block(a).is_ok() && bad(s.insert(l, sid, &ins(a))), "without its inserts it goes, its id is dead");
    let mut st = Style::new(0xFF00_0000);
    st.transparency = Transparency::Percent(91);
    t.ok(matches!(s.add_style(&st), Err(GpuError::Invalid(_))), "a transparency above 90 % is refused");
    let mut ly = Layer::new(0);
    ly.transparency = 91;
    t.ok(matches!(s.add_layer(&ly), Err(GpuError::Invalid(_))), "a layer's transparency above 90 % is refused");
}

// ---------------------------------------------------------------- transparency

pub fn transparency(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = transparency_on(t, dev, level); });
}

fn transparency_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (360u32, 270u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    let bd = transparency_drawing([0, 40, 90]);
    let what = format!("{level}: transparency by layer (0, 40, 90 %), own (30, 20, 0 %), by block (60 %, by the insert's layer)");
    let img = block_image(t, dev, &mut r, &mut off, &bd, &cam, &what)?;
    let ex = expand_blocks(&bd);
    let want = gpu_image(t, dev, &mut r, &mut off, &ex, &cam, &what)?;
    dump(&format!("{}_gpu", name_of(&what)), &img, w, h);
    dump(&format!("{}_want", name_of(&what)), &want, w, h);
    let (n, m) = apart(&img, &want, 2);
    t.ok(n == 0, &format!("{what}: against the drawing with each transparency multiplied into its colour's alpha: {n} pixels more than 2 codes apart, largest {m}"));
    // (against lib/gfx as near as the expected image is: round caps and joins against polygons, translucent strokes that
    // cross themselves blended per piece)
    let reference = refimg::draw(&ex, &cam, (w, h), BG);
    let cp = refimg::compare(&want, &reference, w, BG, 12);
    let ink_p = (cp.ink[0] - cp.ink[1]).abs() / cp.ink[1].max(1.0);
    let tol = Tol { small: 12, over: cp.over + 16, max: cp.max.saturating_add(8), ink: ink_p + 0.001 };
    let c = refimg::compare(&img, &reference, w, BG, tol.small);
    t.ok(c.fine(&tol), &format!("{what}: against lib/gfx: {} (the expected image: {})", c.text(), cp.text()));
    // a layer's transparency changed: no records, the layer table only
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    t.res(s.set_camera(&cam), "camera")?;
    let built = t.res(build_blocks(&mut s, &bd), "build")?;
    let _ = shot(t, dev, &mut off, &mut r, &mut s, "first frame")?;
    let mut l = Layer::new(1);
    l.transparency = 75;
    t.res(s.set_layer(built.layers[1], &l), "set_layer")?;
    let img = shot(t, dev, &mut off, &mut r, &mut s, "after set_layer")?;
    let info = s.upload_info();
    let want = gpu_image(t, dev, &mut r, &mut off, &expand_blocks(&transparency_drawing([0, 75, 90])), &cam, "75 %")?;
    let (n, m) = apart(&img, &want, 2);
    t.ok(n == 0 && info.records == 0 && !info.rebuild && info.tables,
         &format!("{level}: a layer's transparency 40 -> 75 % (set_layer): {} records sent, the layer table {}; against the expected image {n} pixels more than 2 codes apart, largest {m}", info.records, info.tables));
    // no transparency anywhere: the drawing as without transparency (the colours untouched)
    let zero = transparency_drawing([0, 0, 0]);
    let mut flat = zero.clone();
    for (_, st, _) in &mut flat.items {
        st.transparency = Transparency::Percent(0);
    }
    for b in &mut flat.blocks {
        for (st, _) in &mut b.items {
            st.transparency = Transparency::Percent(0);
        }
    }
    let a = block_image(t, dev, &mut r, &mut off, &flat, &cam, "0 %")?;
    let b = gpu_image(t, dev, &mut r, &mut off, &expand_blocks(&flat), &cam, "0 %")?;
    let (n, m) = apart(&a, &b, 2);
    t.ok(n == 0, &format!("{level}: every transparency 0 %: against the colours as they are, {n} pixels more than 2 codes apart, largest {m}"));
    Some(())
}
