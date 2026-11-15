// bench -- a first measurement of render2d (G6a1): N segments offscreen (polylines of 2 to 33 points, a random walk
// over the target at 1 unit per pixel), per level and width: inserting them, the first frame (everything goes up),
// frames with everything in view and zoomed in (a tenth in view), frames with 1000 entities changed each
//
//   render2dtest --bench N [--res 1080p|4k|WxH] [--frames F] [--basis] [--scene cad [--plain | --g6a2b | --g6b1 | --g6b2 | --expand | --no-text]]
//   render2dtest --bench N --scene estate [--inserts K]   a block of about N records (a house) inserted K times (10 000)
//   render2dtest --bench N --scene cad --g6d [..]          G6d: the index (build, memory), picks, windows and snaps per view
//                (CPU), then each view's frame with nothing highlighted against 100 000 entities selected, one hovered and
//                a full overlay (rounds in turn), what the highlight's change sends
//
// GPU times are the scene's timestamps (FrameStats): uploads, visibility (the three compute passes), the draw; each
// frame is waited for before the next (no frames in flight: the times are the frame's own), medians over F frames.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Cap, EntityId, Join, Layer, Region, Renderer, RendererDesc, Scene, SelectMode, Shape, SnapOptions, Style};

use crate::cad;
use crate::gallery::{bg_clear, Rng, BG};
use crate::tests::out;

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    v[v.len() / 2]
}

struct Target {
    color: Texture,
    used: bool,
}

/// one frame, waited for: (prepare's CPU ms, the frame's stamps relative to its start: uploads, visibility, draw ms,
/// records visible)
fn frame(dev: &Device, r: &mut Renderer, s: &mut Scene, t: &mut Target, size: (u32, u32)) -> Result<(f64, [f64; 3], u64)> {
    let t0 = plat::time_us();
    s.prepare(r, size)?;
    let prep = (plat::time_us() - t0) as f64 / 1000.0;
    let mut e = dev.encoder()?;
    let color = ColorAttachment { view: t.color.view(), load: Load::Clear, clear: bg_clear(BG), store: true, before: if t.used { Layout::ColorTarget } else { Layout::Undefined },
                                  after: Layout::ColorTarget };
    r.record(&mut e, s, color)?;
    let id = dev.queue().submit([e.finish()?])?;
    s.submitted(id);
    t.used = true;
    if !dev.wait(id, 30_000_000_000)? {
        return Err(GpuError::Timeout);
    }
    s.collect_stats();
    let st = s.take_stats();
    let f = st.last().copied();
    let (stamps, vis) = match f {
        Some(f) => (f.stamps.map_or([0.0; 3], |v| [v[1] - v[0], v[2] - v[1], v[3] - v[2]]), f.visible),
        None => ([0.0; 3], 0),
    };
    Ok((prep, stamps, vis))
}

pub fn bench(n: u32) -> i32 {
    let size = match plat::arg_value("--res").as_deref() {
        Some("4k") => (3840u32, 2160u32),
        Some(v) if v.contains('x') => {
            let mut it = v.split('x').filter_map(|x| x.parse().ok());
            (it.next().unwrap_or(1920), it.next().unwrap_or(1080))
        }
        _ => (1920, 1080),
    };
    let frames: usize = plat::arg_value("--frames").and_then(|v| v.parse().ok()).unwrap_or(40);
    let basis_only = plat::args().iter().any(|a| a == "--basis");
    let Ok(inst) = Instance::new() else {
        out("SKIP: no GPU");
        return 0;
    };
    let Ok(adapter) = inst.adapter() else {
        out("SKIP: no adapter");
        return 0;
    };
    let cad_scene = plat::arg_value("--scene").as_deref() == Some("cad");
    let estate = plat::arg_value("--scene").as_deref() == Some("estate");
    out(&format!("render2d bench: {n} segments, {} x {}, {frames} frames each, {}{}", size.0, size.1, adapter.info().name, if cad_scene { ", the town plan" } else { "" }));
    let mut levels = Vec::new();
    if adapter.level() == Level::Voll && !basis_only {
        levels.push(Level::Voll);
    }
    levels.push(Level::Basis);
    for level in levels {
        let dev = match adapter.create_device(&DeviceDesc { max_level: level }) {
            Ok(d) => d,
            Err(e) => {
                out(&format!("FAIL device: {e}"));
                return 1;
            }
        };
        let result = if cad_scene || estate { run_cad(&dev, n, size, frames, level, estate) } else { run(&dev, n, size, frames, level) };
        if let Err(e) = result {
            out(&format!("FAIL {level:?}: {e}"));
            return 1;
        }
    }
    0
}

/// the measuring scene's stage: --plain the drawing of G6a2a, --g6a2b that of G6a2b, --g6b1 that of G6b1 (fills, hatches,
/// curves), --g6b2 that of G6b2 (blocks), else with text (G6c; --no-text: the same without it)
pub fn cad_mode() -> cad::Mode {
    let args = plat::args();
    if args.iter().any(|a| a == "--plain") {
        cad::Mode::Plain
    } else if args.iter().any(|a| a == "--g6a2b") {
        cad::Mode::Arcs
    } else if args.iter().any(|a| a == "--g6b1") {
        cad::Mode::Fills
    } else if args.iter().any(|a| a == "--expand") {
        cad::Mode::Expanded
    } else if args.iter().any(|a| a == "--g6b2") {
        cad::Mode::Blocks
    } else if args.iter().any(|a| a == "--no-text") {
        cad::Mode::NoText
    } else if args.iter().any(|a| a == "--text-mixed") {
        cad::Mode::TextMixed
    } else {
        cad::Mode::Text
    }
}

/// what a scene's buffers hold (MB): the pool, the visible list, the instances (their words and spaces); the instance
/// numbers
fn memory(s: &Scene) -> String {
    let ii = s.instance_info();
    let mb = |b: u64| b as f64 / 1048576.0;
    format!("pool {:.1} MB ({} records; of blocks {} in {} variants of {} blocks), visible list {:.1} MB, instances {} ({:.2} MB), entries {}", mb(s.pool_buffer().size()),
            s.pool_buffer().size() / 64, ii.block_records, ii.variants, ii.blocks, mb(s.visible_buffer().size()), ii.instances, mb(ii.instances as u64 * 240), ii.entries)
}

/// the town plan (cad.rs): hairlines and the plan's widths, each at the overview, a district and the working zoom; estate: a
/// block of about n records inserted --inserts times (cad::estate)
fn run_cad(dev: &Device, n: u32, size: (u32, u32), frames: usize, level: Level, estate: bool) -> Result<()> {
    let mut r = Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm))?;
    let inserts: u32 = plat::arg_value("--inserts").and_then(|v| v.parse().ok()).unwrap_or(10_000);
    for (wname, styled) in [("hairlines", false), ("styled", true)] {
        let mut s = Scene::new(&r)?;
        s.set_stats(true);
        let t0 = plat::time_us();
        let mode = cad_mode();
        let c = if estate { cad::estate(&mut s, inserts, styled)? } else { cad::build(&mut s, n, 5, styled, mode)? };
        let insert_ms = (plat::time_us() - t0) as f64 / 1000.0;
        if estate {
            out(&format!("{level:?}, {wname}: a block of {} records inserted {} times: {} records drawn in all", c.insert_records / c.inserts.max(1), c.inserts, c.insert_records));
        } else if mode != cad::Mode::Plain {
            out(&format!("{level:?}, {wname}: of them {} arcs and circles ({} pieces), {} dashed entities ({} records), {} fills ({} hatched; {} records), {} curves ({} records); {} blocks inserted {} times \
                          ({} records drawn); {} records in the pool",
                         c.arcs, c.arc_records, c.dashed, c.dashed_records, c.fills, c.hatched, c.fill_records, c.curves, c.curve_records, c.blocks, c.inserts, c.insert_records, c.segments + c.arcs));
        }
        if c.texts > 0 {
            let ti = s.text_info();
            out(&format!("{level:?}, {wname}: {} texts of {} characters ({} records: glyphs and bars); {} glyphs, {} tiles S, {} pages", c.texts, c.chars, c.text_records, ti.glyphs, ti.tiles_s,
                         ti.pages));
        }
        let mut t = Target { color: dev.create_texture(&TextureDesc::d2(size.0, size.1, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET))?, used: false };
        let views = cad::views(&c, size);
        s.set_camera(&views[0].cam)?;
        let (prep, st0, vis0) = frame(dev, &mut r, &mut s, &mut t, size)?;
        let up = s.upload_info();
        out(&format!("{level:?}, {wname}: {} entities, {} segments, {:.0} x {:.0} m; insert {insert_ms:.1} ms (CPU); first frame: prepare {prep:.1} ms (CPU, {:.1} MB up), GPU uploads {:.3} ms, \
                      visibility {:.3} ms, draw {:.3} ms ({vis0} visible)", c.entities, c.segments, c.width, c.height, up.bytes as f64 / 1048576.0, st0[0], st0[1], st0[2]));
        out(&format!("{level:?}, {wname}: {}", memory(&s)));
        if let Some(dir) = plat::arg_value("--shot") {
            // (each view once as an image: DIR/cad_LEVEL_WIDTHS_VIEW.bgra, as tests::dump writes them)
            let mut off = crate::gallery::Off::new(dev, size.0, size.1)?;
            for (k, v) in views.iter().enumerate() {
                s.set_camera(&v.cam)?;
                let id = off.submit(&mut r, &mut s)?;
                dev.wait(id, 30_000_000_000)?;
                let img = off.read()?;
                let mut f = Vec::with_capacity(img.len() + 8);
                f.extend_from_slice(&size.0.to_le_bytes());
                f.extend_from_slice(&size.1.to_le_bytes());
                f.extend_from_slice(&img);
                let _ = plat::write_file(&format!("{dir}/cad_{level:?}_{wname}_{k}.bgra"), &f);
            }
        }
        for v in &views {
            let (mut a, mut b, mut d, mut p) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut vis = 0;
            for k in 0..frames + 3 {
                // (the camera moves a little: a frame of a pan)
                let mut cam = v.cam;
                cam.center[0] += k as f64 * 0.37 * cam.scale;
                s.set_camera(&cam)?;
                let (prep, st, n) = frame(dev, &mut r, &mut s, &mut t, size)?;
                if k >= 3 {
                    a.push(st[0]);
                    b.push(st[1]);
                    d.push(st[2]);
                    p.push(prep);
                    vis = n;
                }
            }
            let (ma, mb, mc, mp) = (median(&mut a), median(&mut b), median(&mut d), median(&mut p));
            out(&format!("{level:?}, {wname}, {} (scale {:.3}): GPU per frame {:.3} ms (uploads {ma:.3}, visibility {mb:.3}, draw {mc:.3}); prepare {mp:.3} ms CPU; {vis} visible",
                         v.name, v.cam.scale, ma + mb + mc));
        }
        if plat::args().iter().any(|a| a == "--g6d") {
            g6d(dev, &mut r, &mut s, &mut t, size, frames, &views, level, wname, c.trees.first().map(|x| (x.1.block, c.trees.len())))?;
        }
        // (blocks) 1000 inserts moved per frame, at the district: their transforms alone go up
        if c.trees.len() >= 1000 {
            s.set_camera(&views[1].cam)?;
            let mut ids = c.trees.clone();
            let (mut a, mut p, mut g, mut bytes, mut sent, mut rec) = (Vec::new(), Vec::new(), Vec::new(), 0u64, 0u32, 0u32);
            for k in 0..frames + 3 {
                let t0 = plat::time_us();
                for j in 0..1000usize {
                    let i = (k * 7919 + j * 104_729) % ids.len();
                    let (id, ref mut ins) = ids[i];
                    ins.at[0] += 0.05;
                    ins.rotation += 0.01;
                    s.update(id, &render2d::Shape::Insert(*ins))?;
                }
                let upd = (plat::time_us() - t0) as f64 / 1000.0;
                let (prep, st, _) = frame(dev, &mut r, &mut s, &mut t, size)?;
                if k >= 3 {
                    let (info, ii) = (s.upload_info(), s.instance_info());
                    bytes = ii.sent_bytes;
                    sent = ii.sent_instances;
                    rec = info.records;
                    a.push(st[0]);
                    p.push(prep + upd);
                    g.push(st[0] + st[1] + st[2]);
                }
            }
            out(&format!("{level:?}, {wname}, 1000 inserts moved per frame at the district ({sent} instances, {:.3} MB, {rec} records): update + prepare {:.3} ms CPU, GPU uploads {:.3} ms, frame {:.3} ms",
                         bytes as f64 / 1048576.0, median(&mut p), median(&mut a), median(&mut g)));
        }
        dev.wait_idle()?;
    }
    Ok(())
}

/// percentiles of a list of times (ms): median, 99th, largest
fn spread(v: &mut [f64]) -> (f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    (v[v.len() / 2], v[(v.len() * 99 / 100).min(v.len() - 1)], v[v.len() - 1])
}

/// G6d on the measuring scene: the index, the queries' times per view, the frames with and without highlights and an overlay
#[allow(clippy::too_many_arguments)]
fn g6d(dev: &Device, r: &mut Renderer, s: &mut Scene, t: &mut Target, size: (u32, u32), frames: usize, views: &[cad::View], level: Level, wname: &str, block: Option<(render2d::BlockId, usize)>) -> Result<()> {
    // the index in one go (the first query builds it)
    let t0 = plat::time_us();
    s.build_index();
    let build_ms = (plat::time_us() - t0) as f64 / 1000.0;
    let ii = s.index_info();
    out(&format!("{level:?}, {wname}, G6d index: built in {build_ms:.1} ms CPU; {} entries in {} nodes, height {}, {:.2} MB; {} block trees ({} entries, {:.3} MB); {} looked at by every query",
                 ii.entries, ii.nodes, ii.height, ii.bytes as f64 / 1048576.0, ii.block_trees, ii.block_entries, ii.block_bytes as f64 / 1048576.0, ii.outside));
    // what a change costs the index: 1000 lines inserted, moved, removed (each set applied by the next query), a block's change
    {
        let c0 = views[0].cam.center;
        let lay = s.drawn_layers()[0];
        let st = s.add_style(&Style::new(0xFF20_2020))?;
        let mut ids = Vec::with_capacity(1000);
        let mut rng = crate::gallery::Rng(3);
        let line = |rng: &mut crate::gallery::Rng| {
            let a = [c0[0] + rng.range(-1500.0, 1500.0), c0[1] + rng.range(-800.0, 800.0)];
            Shape::Line { a, b: [a[0] + rng.range(-5.0, 5.0), a[1] + rng.range(-5.0, 5.0)] }
        };
        for _ in 0..1000 {
            ids.push(s.insert(lay, st, &line(&mut rng))?);
        }
        let t0 = plat::time_us();
        let _ = s.pick(c0, 1.0);
        let ins_ms = (plat::time_us() - t0) as f64 / 1000.0;
        for &id in &ids {
            s.update(id, &line(&mut rng))?;
        }
        let t0 = plat::time_us();
        let _ = s.pick(c0, 1.0);
        let upd_ms = (plat::time_us() - t0) as f64 / 1000.0;
        for &id in &ids {
            s.remove(id)?;
        }
        let t0 = plat::time_us();
        let _ = s.pick(c0, 1.0);
        let rem_ms = (plat::time_us() - t0) as f64 / 1000.0;
        out(&format!("{level:?}, {wname}, G6d index per change (1000 each, applied by the next query): insert {:.2} us, update {:.2} us, remove {:.2} us", ins_ms, upd_ms, rem_ms));
        // a block inserted many times gets an entity more: its tree and every insert's box again with the next query
        if let Some((b, n)) = block {
            let id = s.insert_in(b, st, &Shape::Line { a: [0.0, 0.0], b: [0.3, 0.2] })?;
            let t0 = plat::time_us();
            let _ = s.pick(c0, 1.0);
            let bms = (plat::time_us() - t0) as f64 / 1000.0;
            let info = s.index_info();
            s.remove(id)?;
            let _ = s.pick(c0, 1.0);
            out(&format!("{level:?}, {wname}, G6d: a block of {n} inserts changed: the next query {bms:.2} ms ({} blocks made again, {} entries updated)", info.blocks_synced, info.synced));
        }
    }
    // queries per view: picks (3 px), snaps (10 px, all kinds and a base point), a window and a crossing of a quarter of the view
    let mut rng = crate::gallery::Rng(11);
    for v in views {
        s.set_camera(&v.cam)?;
        let (mut pk, mut sn, mut hits) = (Vec::new(), Vec::new(), 0usize);
        let opt = SnapOptions::new(10.0, render2d::SNAP_ALL);
        for k in 0..600 {
            let q = v.cam.to_drawing([rng.range(0.0, size.0 as f64), rng.range(0.0, size.1 as f64)], size);
            let t0 = plat::time_us();
            hits += s.pick(q, 3.0).len();
            pk.push((plat::time_us() - t0) as f64 / 1000.0);
            let o = if k % 2 == 0 { opt.with_base(v.cam.to_drawing([rng.range(0.0, size.0 as f64), rng.range(0.0, size.1 as f64)], size)) } else { opt };
            let t0 = plat::time_us();
            let _ = s.snap(q, &o);
            sn.push((plat::time_us() - t0) as f64 / 1000.0);
        }
        let (pm, p99, px) = spread(&mut pk);
        let (sm, s99, sx) = spread(&mut sn);
        let a = v.cam.to_drawing([0.25 * size.0 as f64, 0.25 * size.1 as f64], size);
        let b = v.cam.to_drawing([0.75 * size.0 as f64, 0.75 * size.1 as f64], size);
        let t0 = plat::time_us();
        let w = s.select(Region::Rect(a, b), SelectMode::Window).len();
        let wt = (plat::time_us() - t0) as f64 / 1000.0;
        let t0 = plat::time_us();
        let cr = s.select(Region::Rect(a, b), SelectMode::Crossing).len();
        let ct = (plat::time_us() - t0) as f64 / 1000.0;
        out(&format!("{level:?}, {wname}, {} G6d queries: pick (3 px) median {pm:.4} ms, 99 % {p99:.4}, most {px:.4} ({hits} hits in 600); snap (10 px, all kinds) median {sm:.4} ms, 99 % {s99:.4}, \
                      most {sx:.4}; a window of the view's middle quarter {wt:.3} ms ({w}), a crossing {ct:.3} ms ({cr})", v.name));
    }
    // a window over the whole overview
    s.set_camera(&views[0].cam)?;
    let a = views[0].cam.to_drawing([0.0, 0.0], size);
    let b = views[0].cam.to_drawing([size.0 as f64, size.1 as f64], size);
    let t0 = plat::time_us();
    let all = s.select(Region::Rect([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]), SelectMode::Window);
    let wall = (plat::time_us() - t0) as f64 / 1000.0;
    let t0 = plat::time_us();
    let all2 = s.select(Region::Rect([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]), SelectMode::Crossing);
    let call = (plat::time_us() - t0) as f64 / 1000.0;
    out(&format!("{level:?}, {wname}, G6d: a window over the whole overview {wall:.2} ms ({} entities), a crossing {call:.2} ms ({})", all.len(), all2.len()));
    // where the snap's time goes in the overview: the same 600 points with fewer kinds
    {
        let v = &views[0];
        let mut rng = crate::gallery::Rng(17);
        let pts: Vec<([f64; 2], [f64; 2])> = (0..600)
            .map(|_| {
                let q = v.cam.to_drawing([rng.range(0.0, size.0 as f64), rng.range(0.0, size.1 as f64)], size);
                (q, v.cam.to_drawing([rng.range(0.0, size.0 as f64), rng.range(0.0, size.1 as f64)], size))
            })
            .collect();
        let all_k = render2d::SNAP_ALL;
        let sets = [("all kinds, a base point", all_k, true), ("all kinds", all_k, false), ("without intersections", all_k & !render2d::SNAP_INT, false),
                    ("endpoints only", render2d::SNAP_END, false)];
        let mut line = String::new();
        for (name, kinds, with_base) in sets {
            let mut ts = Vec::new();
            let mut n = 0;
            for &(q, b) in &pts {
                let o = SnapOptions::new(10.0, kinds);
                let o = if with_base { o.with_base(b) } else { o };
                let t0 = plat::time_us();
                n += s.snaps(q, &o).len();
                ts.push((plat::time_us() - t0) as f64 / 1000.0);
            }
            let (m, p99, mx) = spread(&mut ts);
            line += &format!("; {name}: median {m:.4} ms, 99 % {p99:.3}, most {mx:.3} ({n} candidates)");
        }
        out(&format!("{level:?}, {wname}, overview G6d snaps (10 px, 600 points){line}"));
    }
    // the scene's changes above (lines, a block) go up first: what follows is the highlight alone
    let _ = frame(dev, r, s, t, size)?;
    // 100 000 selected: what goes up, CPU; then the frames with and without, rounds in turn
    let sel: Vec<EntityId> = all.iter().copied().take(100_000).collect();
    let t0 = plat::time_us();
    s.set_highlights(&sel, render2d::HL_SELECTED)?;
    let set_ms = (plat::time_us() - t0) as f64 / 1000.0;
    let (prep, st, _) = frame(dev, r, s, t, size)?;
    let hi = s.highlight_info();
    out(&format!("{level:?}, {wname}, G6d: {} entities selected, the first time (the highlight's pipelines made): set_highlights {set_ms:.2} ms CPU, prepare {prep:.2} ms CPU, {} of {} words up ({:.3} MB, {} runs), GPU uploads {:.3} ms, {} records",
                 sel.len(), hi.sent_words, hi.words, hi.sent_words as f64 * 4.0 / 1048576.0, hi.runs, st[0], s.upload_info().records));
    s.clear_highlights(render2d::HL_SELECTED | render2d::HL_HOVER);
    let _ = frame(dev, r, s, t, size)?;
    let t0 = plat::time_us();
    s.set_highlights(&sel, render2d::HL_SELECTED)?;
    let set_ms = (plat::time_us() - t0) as f64 / 1000.0;
    let (prep, st, _) = frame(dev, r, s, t, size)?;
    let hi = s.highlight_info();
    out(&format!("{level:?}, {wname}, G6d: {} entities selected again: set_highlights {set_ms:.2} ms CPU, prepare {prep:.2} ms CPU, {} of {} words up ({:.3} MB, {} runs), GPU uploads {:.3} ms, {} records",
                 sel.len(), hi.sent_words, hi.words, hi.sent_words as f64 * 4.0 / 1048576.0, hi.runs, st[0], s.upload_info().records));
    // the hover moving from entity to entity: what goes up
    let mut hv = Vec::new();
    let mut words = 0;
    for k in 0..20 {
        if k > 0 {
            s.set_highlight(sel[(k - 1) * 997 % sel.len()], render2d::HL_SELECTED)?;
        }
        s.set_highlight(sel[k * 997 % sel.len()], render2d::HL_SELECTED | render2d::HL_HOVER)?;
        let t0 = plat::time_us();
        let _ = frame(dev, r, s, t, size)?;
        hv.push((plat::time_us() - t0) as f64 / 1000.0);
        words = words.max(s.highlight_info().sent_words);
    }
    out(&format!("{level:?}, {wname}, G6d: the hover moved per frame: at most {words} words up"));
    for v in views {
        // rounds in turn: nothing, the overlay alone, the selection alone, both
        let mut gpu: [Vec<f64>; 4] = Default::default();
        let mut cpu: [Vec<f64>; 4] = Default::default();
        for round in 0..12 {
            let st_k = round % 4;
            let (lit, ov) = (st_k >= 2, st_k % 2 == 1);
            if lit {
                s.set_highlights(&sel, render2d::HL_SELECTED)?;
            } else {
                s.clear_highlights(render2d::HL_SELECTED | render2d::HL_HOVER);
            }
            s.overlay().clear();
            for k in 0..frames / 2 + 3 {
                let mut cam = v.cam;
                cam.center[0] += k as f64 * 0.37 * cam.scale;
                s.set_camera(&cam)?;
                if ov {
                    cad::g6d_overlay(s, k, v);
                }
                let (p, st, _) = frame(dev, r, s, t, size)?;
                if k >= 3 {
                    gpu[st_k].push(st[0] + st[1] + st[2]);
                    cpu[st_k].push(p);
                }
            }
        }
        cad::g6d_overlay(s, 0, v);
        let oi = s.overlay_info();
        s.overlay().clear();
        let m: Vec<(f64, f64)> = (0..4).map(|k| (median(&mut gpu[k]), median(&mut cpu[k]))).collect();
        out(&format!("{level:?}, {wname}, {} G6d frames: nothing highlighted {:.3} ms GPU (prepare {:.3} ms CPU); a full overlay ({} records, {:.1} KB) {:.3} ms GPU (prepare {:.3} ms CPU); \
                      {} selected {:.3} ms GPU (prepare {:.3} ms CPU); both {:.3} ms GPU (prepare {:.3} ms CPU)",
                     v.name, m[0].0, m[0].1, oi.records, oi.bytes as f64 / 1024.0, m[1].0, m[1].1, sel.len(), m[2].0, m[2].1, m[3].0, m[3].1));
    }
    s.clear_highlights(render2d::HL_SELECTED | render2d::HL_HOVER);
    s.overlay().clear();
    Ok(())
}

fn run(dev: &Device, n: u32, size: (u32, u32), frames: usize, level: Level) -> Result<()> {
    let mut r = Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm))?;
    for (wname, width) in [("hairlines", 0.0f32), ("3 px, round joins", 3.0)] {
        let mut s = Scene::new(&r)?;
        s.set_stats(true);
        let l = s.add_layer(&Layer::new(0))?;
        let mut st = Style::new(0xFF30_4050);
        st.width = width;
        if width > 0.0 {
            st.cap = Cap::Round;
            st.join = Join::Round;
        }
        let sid = s.add_style(&st)?;
        let (w, h) = (size.0 as f64, size.1 as f64);
        let mut rng = Rng(5);
        let mut polys: Vec<(EntityId, Vec<[f64; 2]>)> = Vec::new();
        let mut segs = 0u32;
        let mut pts: Vec<[f64; 2]> = Vec::new();
        let t0 = plat::time_us();
        while segs < n {
            let m = (2 + rng.below(32)).min(n - segs + 1) as usize;
            pts.clear();
            pts.push([rng.range(-0.5 * w, 0.5 * w), rng.range(-0.5 * h, 0.5 * h)]);
            for _ in 1..m {
                let q = pts[pts.len() - 1];
                pts.push([q[0] + rng.range(-12.0, 12.0), q[1] + rng.range(-12.0, 12.0)]);
            }
            let id = s.insert(l, sid, &Shape::Polyline { points: &pts, closed: false })?;
            if polys.len() < 1 << 16 {
                polys.push((id, pts.clone()));
            }
            segs += m as u32 - 1;
        }
        let insert_ms = (plat::time_us() - t0) as f64 / 1000.0;
        let mut t = Target { color: dev.create_texture(&TextureDesc::d2(size.0, size.1, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET))?, used: false };
        s.set_camera(&Camera::new([0.0, 0.0], 1.0))?;
        // the first frame: every record goes up
        let (prep, st0, vis0) = frame(dev, &mut r, &mut s, &mut t, size)?;
        let up = s.upload_info();
        out(&format!("{level:?}, {wname}: {} entities, {segs} segments; insert {insert_ms:.1} ms (CPU); first frame: prepare {prep:.1} ms (CPU, {:.1} MB up), GPU uploads {:.3} ms, \
                      visibility {:.3} ms, draw {:.3} ms ({vis0} visible)", s.len(), up.bytes as f64 / 1048576.0, st0[0], st0[1], st0[2]));
        // everything in view, then a tenth of it
        for (vname, scale) in [("all in view", 1.0f64), ("zoomed in (a tenth in view)", 0.316)] {
            s.set_camera(&Camera::new([0.0, 0.0], scale))?;
            let (mut a, mut b, mut c, mut p) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut vis = 0;
            for k in 0..frames + 3 {
                // (the camera moves a little: a frame of a pan)
                s.set_camera(&Camera::new([k as f64 * 0.37, 0.0], scale))?;
                let (prep, st, v) = frame(dev, &mut r, &mut s, &mut t, size)?;
                if k >= 3 {
                    a.push(st[0]);
                    b.push(st[1]);
                    c.push(st[2]);
                    p.push(prep);
                    vis = v;
                }
            }
            let (ma, mb, mc, mp) = (median(&mut a), median(&mut b), median(&mut c), median(&mut p));
            out(&format!("{level:?}, {wname}, {vname}: GPU per frame {:.3} ms (uploads {ma:.3}, visibility {mb:.3}, draw {mc:.3}); prepare {mp:.3} ms CPU; {vis} visible",
                         ma + mb + mc));
        }
        // 1000 entities changed per frame
        s.set_camera(&Camera::new([0.0, 0.0], 1.0))?;
        let (mut a, mut p, mut bytes) = (Vec::new(), Vec::new(), 0u64);
        let mut rec = 0u32;
        for k in 0..frames {
            let t0 = plat::time_us();
            for j in 0..1000usize {
                let i = (k * 7919 + j * 104_729) % polys.len();
                let (id, ref mut q) = polys[i];
                for v in q.iter_mut() {
                    v[0] += 0.5;
                }
                s.update(id, &Shape::Polyline { points: q, closed: false })?;
            }
            let upd = (plat::time_us() - t0) as f64 / 1000.0;
            let (prep, st, _) = frame(dev, &mut r, &mut s, &mut t, size)?;
            let info = s.upload_info();
            bytes = info.bytes;
            rec = info.records;
            a.push(st[0]);
            p.push(prep + upd);
        }
        out(&format!("{level:?}, {wname}, 1000 entities changed per frame ({rec} records, {:.2} MB): update + prepare {:.3} ms CPU, GPU uploads {:.3} ms",
                     bytes as f64 / 1048576.0, median(&mut p), median(&mut a)));
        dev.wait_idle()?;
    }
    Ok(())
}
