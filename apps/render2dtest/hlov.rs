// hlov -- render2dtest's tests of highlighting and the overlay (G6d; main.rs lists them)
//
//   highlight  selected and hovered entities of every kind (strokes in pixels and units, an arc, a dash pattern, a fill, a
//              hatch, text, an insert): the image = the same drawing with those entities restyled as the highlight looks
//              (hl.wgsl's colour and width computed here), byte for byte; everything else byte for byte as without a
//              highlight; cleared: the image of before; what goes up (the mask's words, no record); Voll = Basis
//   overlay    rubber bands in drawing units and pixels, window and crossing boxes, grips, every snap marker, a filled
//              polygon: against the same geometry as plain entities (GPU) and against lib/gfx; an empty overlay changes no
//              pixel and sends nothing; a rubber band to a point at 1e7 seen at 1e-6 units per pixel where the camera puts it

#[cfg(not(target_arch = "wasm32"))]
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(not(target_arch = "wasm32"))]
use gpu::*;
use render2d::{Cap, Join, Layer, Scene, SnapKind, Space, Style, Unit};
#[cfg(not(target_arch = "wasm32"))]
use render2d::{Camera, Renderer, RendererDesc, Shape};

use crate::gallery::*;
#[cfg(not(target_arch = "wasm32"))]
use crate::refimg::{self, Tol};
#[cfg(not(target_arch = "wasm32"))]
use crate::tests::{dump, gpu_image, on_levels, shot, T};

#[cfg(not(target_arch = "wasm32"))]
/// hl.wgsl's colour on the CPU: selected 3/4 of the way to blue, hovered 7/8 of the way to orange, alpha at least 0xB0
pub fn hl_ink(c: u32, bits: u8) -> u32 {
    if bits == 0 {
        return c;
    }
    let a = (c >> 24).max(render2d::HL_ALPHA);
    let (t, k) = if bits & render2d::HL_HOVER != 0 { (render2d::HL_HOVER_RGB, 224u32) } else { (render2d::HL_SELECTED_RGB, 192u32) };
    let ch = |s: u32| ((((c >> s) & 255) * (256 - k) + ((t >> s) & 255) * k + 128) >> 8) << s;
    (a << 24) | ch(16) | ch(8) | ch(0)
}

#[cfg(not(target_arch = "wasm32"))]
/// a style as the highlight draws it at a camera's scale: hl_ink's colour, the half width in pixels (as the vertex shader
/// has it: at least half a pixel) wider by a quarter pixel (selected) or a pixel (hovered)
pub fn hl_style(st: &Style, bits: u8, scale: f64) -> Style {
    let mut s = *st;
    s.color = hl_ink(st.color, bits);
    if bits == 0 {
        return s;
    }
    let wider = if bits & render2d::HL_HOVER != 0 { 1.0f32 } else { 0.25 };
    let rest_z = (1.0 / scale) as f32;
    let w = if st.unit == Unit::Drawing { st.width * rest_z } else { st.width };
    let h = (0.5 * w).max(0.5) + wider;
    s.width = 2.0 * h;
    s.unit = Unit::Px;
    s
}

/// the highlight test's drawing (two layers) and which of its items are selected (1) or hovered (2)
pub fn hl_drawing() -> (Drawing, Vec<u8>) {
    let mut d = Drawing { layers: vec![(true, 0), (true, 1)], items: Vec::new() };
    let mut bits = Vec::new();
    let mut add = |layer: usize, st: Style, geo: Geo, b: u8| {
        d.items.push(Item { layer, style: st, geo });
        bits.push(b);
    };
    add(0, style(INK[0], 0.0, Cap::Butt, Join::Miter), Geo::Poly(vec![[20.5, 20.5], [120.0, 40.0], [60.0, 90.0]], false), 1);
    add(0, style(INK[1], 3.0, Cap::Round, Join::Round), Geo::Line([20.0, 120.0], [150.0, 130.0]), 2);
    add(0, style(INK[2], 1.0, Cap::Butt, Join::Miter), Geo::Line([20.0, 140.0], [150.0, 150.0]), 0);
    let mut units = style(INK[3], 0.0, Cap::Square, Join::Bevel);
    units.unit = Unit::Drawing;
    units.width = 5.0;
    add(0, units, Geo::Poly(vec![[180.0, 20.0], [260.0, 60.0], [200.0, 100.0], [300.0, 110.0]], false), 1);
    add(0, style(INK[0], 2.0, Cap::Butt, Join::Miter), Geo::Arc([250.0, 180.0], 40.0, 0.3, 3.5), 2);
    add(1, dashed(INK[1], 2.0, Cap::Round, Join::Round, &[8.0, 4.0], Unit::Px, 0.0), Geo::Poly(vec![[30.0, 200.0], [150.0, 240.0], [60.0, 255.0]], false), 1);
    add(1, style(0xC0E0_A040, 0.0, Cap::Butt, Join::Miter), Geo::Fill(vec![vec![Geo::Poly(vec![[200.0, 200.0], [330.0, 205.0], [320.0, 260.0], [190.0, 250.0]], true)], vec![Geo::Circle([260.0, 228.0], 12.0)]]), 1);
    let hatch = render2d::HatchLine::new(0.7, 6.0);
    let mut hs = style(INK[2], 0.0, Cap::Butt, Join::Miter).hatched(&[hatch], 1.0);
    hs.width = 0.0;
    add(1, hs, Geo::Fill(vec![vec![Geo::Circle([90.0, 180.0], 22.0)]]), 2);
    add(1, style(INK[3], 1.0, Cap::Butt, Join::Miter), Geo::Circle([320.0, 60.0], 18.0), 0);
    (d, bits)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn highlight(t: &mut T) {
    let mut shots: Vec<(&str, Vec<u8>)> = Vec::new();
    on_levels(t, &mut |t, dev, level| {
        if let Some(v) = highlight_on(t, dev, level) {
            shots.push((if level == "Voll" { "Voll" } else { "Basis" }, v));
        }
    });
    if shots.len() == 2 {
        let (n, _) = differ(&shots[0].1, &shots[1].1);
        t.ok(n == 0, &format!("highlight: Voll and Basis draw the highlighted drawing alike: {n} pixels differ"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn highlight_on(t: &mut T, dev: &Device, level: &str) -> Option<Vec<u8>> {
    let (w, h) = (360u32, 270u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    let (d, bits) = hl_drawing();
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    t.res(s.set_camera(&cam), "camera")?;
    // the drawing, its ids
    let mut layers = Vec::new();
    for &(v, o) in &d.layers {
        let mut l = Layer::new(o);
        l.visible = v;
        layers.push(t.res(s.add_layer(&l), "add_layer")?);
    }
    let mut ids = Vec::new();
    for it in &d.items {
        let st = t.res(s.add_style(&it.style), "add_style")?;
        ids.push(t.res(with_shape(&it.geo, |sh| s.insert(layers[it.layer], st, sh)), "insert")?);
    }
    let before = shot(t, dev, &mut off, &mut r, &mut s, "before")?;
    // highlighted: what went up (no record), the image against the restyled drawing
    for (k, &b) in bits.iter().enumerate() {
        if b != 0 {
            t.res(s.set_highlight(ids[k], b), "set_highlight")?;
        }
    }
    let lit = shot(t, dev, &mut off, &mut r, &mut s, "highlighted")?;
    let up = s.upload_info();
    let hi = s.highlight_info();
    t.ok(up.records == 0 && hi.sent_words > 0 && hi.sent_words <= hi.words, &format!("{level}: highlighting {} entities sends no record ({}), the mask's {} of {} words",
                                                                                    bits.iter().filter(|&&b| b != 0).count(), up.records, hi.sent_words, hi.words));
    let mut restyled = d.clone();
    for (k, it) in restyled.items.iter_mut().enumerate() {
        it.style = hl_style(&it.style, bits[k], cam.scale);
    }
    let want = gpu_image(t, dev, &mut r, &mut off, &restyled, &cam, "restyled")?;
    let (n, m) = differ(&lit, &want);
    dump(&format!("highlight_{level}_gpu"), &lit, w, h);
    dump(&format!("highlight_{level}_want"), &want, w, h);
    t.ok(n == 0, &format!("{level}: selected and hovered strokes (pixels, units), an arc, a dash pattern, a fill with a hole, a hatch: as the drawing restyled to the highlight's colour and width, \
                           {n} pixels differ (largest {m})"));
    // against lib/gfx: the restyled drawing as the other tests compare
    let reference = refimg::draw(&restyled, &cam, (w, h), BG);
    let tol = Tol { small: 12, over: 900, max: 120, ink: 0.01 };
    let c = refimg::compare(&lit, &reference, w, BG, tol.small);
    t.ok(c.fine(&tol), &format!("{level}: the highlighted drawing against lib/gfx: {}", c.text()));
    // the hover moves to another entity: a few words go up; then everything cleared: the image of before, byte for byte
    t.res(s.set_highlight(ids[1], 0), "set_highlight")?;
    t.res(s.set_highlight(ids[2], render2d::HL_HOVER), "set_highlight")?;
    shot(t, dev, &mut off, &mut r, &mut s, "hover moved")?;
    let hi = s.highlight_info();
    t.ok(hi.sent_words <= 4 && hi.runs <= 2 && !hi.full, &format!("{level}: the hover moved from one entity to another: {} words in {} runs went up", hi.sent_words, hi.runs));
    s.clear_highlights(render2d::HL_SELECTED | render2d::HL_HOVER);
    let after = shot(t, dev, &mut off, &mut r, &mut s, "cleared")?;
    let (n, _) = differ(&after, &before);
    t.ok(n == 0, &format!("{level}: every highlight cleared: the image of before, {n} pixels differ"));
    // a highlight on one layer leaves the other byte for byte (its layer: everything but the highlighted entity's pixels)
    t.res(s.set_highlight(ids[8], render2d::HL_SELECTED), "set_highlight")?;
    let one = shot(t, dev, &mut off, &mut r, &mut s, "one highlighted")?;
    let (mut out, mut inside) = (0u32, 0u32);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let k = (y * w as usize + x) * 4;
            if one[k..k + 3] != before[k..k + 3] {
                // the circle about (320, 60) radius 18: rows count down from h
                let (dx, dy) = (x as f64 + 0.5 - 320.0, (h as f64 - y as f64 - 0.5) - 60.0);
                let rr = render2d::math::sqrt(dx * dx + dy * dy);
                if (rr - 18.0).abs() <= 3.0 { inside += 1 } else { out += 1 }
            }
        }
    }
    t.ok(out == 0 && inside > 0, &format!("{level}: one circle selected: {inside} pixels of its ring change, {out} elsewhere"));
    s.clear_highlights(render2d::HL_SELECTED | render2d::HL_HOVER);
    // text and an insert (its block's entities as one)
    let fonts = t.res(crate::textdraw::add_fonts(&mut s), "fonts")?;
    let ts = t.res(s.add_text_style(&render2d::TextStyle::new(fonts[1])), "text style")?;
    let st = t.res(s.add_style(&Style::new(INK[0])), "style")?;
    let mut tx = render2d::Text::new("Hervorheben", ts, [20.0, 160.0]);
    tx.height = Some(14.0);
    let tid = t.res(s.insert(layers[1], st, &Shape::Text(tx)), "text")?;
    let blk = t.res(s.add_block(&render2d::Block::new()), "block")?;
    let bst = t.res(s.add_style(&style(INK[1], 2.0, Cap::Butt, Join::Miter)), "style")?;
    t.res(s.insert_in(blk, bst, &Shape::Circle { center: [0.0, 0.0], radius: 10.0 }), "insert_in")?;
    t.res(s.insert_in(blk, bst, &Shape::Line { a: [-12.0, 0.0], b: [12.0, 0.0] }), "insert_in")?;
    let mut ins = render2d::Insert::new(blk, [180.0, 160.0]);
    ins.columns = 2;
    ins.spacing = [40.0, 0.0];
    let iid = t.res(s.insert(layers[1], st, &Shape::Insert(ins)), "insert")?;
    let plain = shot(t, dev, &mut off, &mut r, &mut s, "with text and an insert")?;
    t.res(s.set_highlight(tid, render2d::HL_HOVER), "set_highlight")?;
    t.res(s.set_highlight(iid, render2d::HL_SELECTED), "set_highlight")?;
    let lit2 = shot(t, dev, &mut off, &mut r, &mut s, "text and insert highlighted")?;
    // the text in the hover's colour, the insert's both copies in the selection's: their pixels change, nothing else
    let (mut changed, mut stray) = (0u32, 0u32);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let k = (y * w as usize + x) * 4;
            if lit2[k..k + 3] != plain[k..k + 3] {
                let (px, py) = (x as f64 + 0.5, h as f64 - y as f64 - 0.5);
                let in_text = (16.0..160.0).contains(&px) && (150.0..180.0).contains(&py);
                let in_ins = (164.0..236.0).contains(&px) && (146.0..174.0).contains(&py);
                if in_text || in_ins { changed += 1 } else { stray += 1 }
            }
        }
    }
    let blue = lit2.chunks_exact(4).filter(|p| p[0] > 160 && p[2] < 120).count();
    t.ok(changed > 100 && stray == 0 && blue > 20, &format!("{level}: a hovered text and a selected array of an insert: {changed} pixels of theirs change ({blue} blue), {stray} elsewhere"));
    Some(lit)
}

// ---------------------------------------------------------------- the overlay

/// the overlay test's items in pixels (x right, y down) and the same as a drawing for lib/gfx (pixel_camera: y up)
fn fill_overlay(s: &mut Scene, h: u32) -> Drawing {
    let mut d = Drawing::one_layer();
    let flip = |p: [f64; 2]| [p[0], h as f64 - p[1]];
    let ov = s.overlay();
    ov.clear();
    let rubber = style(0xFF20_2020, 1.0, Cap::Butt, Join::Miter);
    ov.line([20.0, 20.0], [150.0, 70.0], Space::Px, &rubber);
    d.add(rubber, Geo::Line(flip([20.0, 20.0]), flip([150.0, 70.0])));
    let wide = style(0xC030_70D0, 4.0, Cap::Round, Join::Round);
    ov.polyline(&[[30.0, 100.0], [90.0, 140.0], [40.0, 170.0]], false, Space::Px, &wide);
    d.add(wide, Geo::Poly(vec![flip([30.0, 100.0]), flip([90.0, 140.0]), flip([40.0, 170.0])], false));
    ov.fill(&[[200.0, 20.0], [300.0, 30.0], [260.0, 90.0]], Space::Px, 0x80E0_6020);
    d.add(Style::new(0x80E0_6020), Geo::Fill(vec![vec![Geo::Poly(vec![flip([200.0, 20.0]), flip([300.0, 30.0]), flip([260.0, 90.0])], true)]]));
    ov.circle([300.0, 160.0], 25.0, Space::Px, &rubber);
    d.add(rubber, Geo::Circle(flip([300.0, 160.0]), 25.0));
    // the selection boxes' look: a window (blue tint, solid), a crossing (green tint, dashed)
    ov.selection_box([120.0, 110.0], [200.0, 160.0], Space::Px);
    d.add(Style::new(render2d::WINDOW_FILL), Geo::Fill(vec![vec![Geo::Poly(vec![flip([120.0, 110.0]), flip([200.0, 110.0]), flip([200.0, 160.0]), flip([120.0, 160.0])], true)]]));
    d.add(style(render2d::WINDOW_EDGE, 1.0, Cap::Butt, Join::Miter), Geo::Poly(vec![flip([120.0, 110.0]), flip([200.0, 110.0]), flip([200.0, 160.0]), flip([120.0, 160.0])], true));
    ov.selection_box([200.0, 230.0], [120.0, 180.0], Space::Px);
    d.add(Style::new(render2d::CROSSING_FILL), Geo::Fill(vec![vec![Geo::Poly(vec![flip([200.0, 230.0]), flip([120.0, 230.0]), flip([120.0, 180.0]), flip([200.0, 180.0])], true)]]));
    d.add(dashed(render2d::CROSSING_EDGE, 1.0, Cap::Butt, Join::Miter, &[5.0, 3.0], Unit::Px, 0.0), Geo::Poly(vec![flip([200.0, 230.0]), flip([120.0, 230.0]), flip([120.0, 180.0]), flip([200.0, 180.0])], true));
    // grips and markers (on their pixels' centres): the reference draws their squares and symbols as the overlay builds them
    let mut x = 30.0;
    for g in [render2d::Grip::Cold, render2d::Grip::Hover, render2d::Grip::Hot] {
        ov.grip([x + 0.3, 210.7], Space::Px, g);
        x += 20.0;
    }
    let kinds = [SnapKind::Endpoint, SnapKind::Midpoint, SnapKind::Center, SnapKind::Quadrant, SnapKind::Intersection, SnapKind::Perpendicular, SnapKind::Tangent, SnapKind::Nearest, SnapKind::Insertion];
    for (k, kind) in kinds.iter().enumerate() {
        ov.marker(*kind, [30.5 + 30.0 * k as f64, 245.5], Space::Px);
    }
    d
}

#[cfg(not(target_arch = "wasm32"))]
pub fn overlay(t: &mut T) {
    let mut shots: Vec<Vec<u8>> = Vec::new();
    on_levels(t, &mut |t, dev, level| {
        if let Some(v) = overlay_on(t, dev, level) {
            shots.push(v);
        }
    });
    if shots.len() == 2 {
        let (n, _) = differ(&shots[0], &shots[1]);
        t.ok(n == 0, &format!("overlay: Voll and Basis draw it alike: {n} pixels differ"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn overlay_on(t: &mut T, dev: &Device, level: &str) -> Option<Vec<u8>> {
    let (w, h) = (340u32, 260u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    let base = joins_drawing(Join::Round, 3.0);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    t.res(s.set_camera(&cam), "camera")?;
    t.res(build(&mut s, &base), "build")?;
    let plain = shot(t, dev, &mut off, &mut r, &mut s, "no overlay")?;
    // an empty overlay: no pixel, nothing sent
    s.overlay().clear();
    let again = shot(t, dev, &mut off, &mut r, &mut s, "an empty overlay")?;
    let (n, _) = differ(&plain, &again);
    let oi = s.overlay_info();
    t.ok(n == 0 && oi.bytes == 0 && s.upload_info().records == 0, &format!("{level}: an empty overlay: {n} pixels differ, {} bytes of it sent, {} records", oi.bytes, s.upload_info().records));
    // the overlay alone (over an empty scene) against the same geometry as plain entities and against lib/gfx
    let mut e = t.res(Scene::new(&r), "Scene::new")?;
    t.res(e.set_camera(&cam), "camera")?;
    let d = fill_overlay(&mut e, h);
    let img = shot(t, dev, &mut off, &mut r, &mut e, "the overlay")?;
    let oi = e.overlay_info();
    // (the grips and markers are left out of the comparison: their own rectangle)
    let mask = |_x: usize, y: usize| (y as u32) < 195;
    let as_plain = gpu_image(t, dev, &mut r, &mut off, &d, &cam, "the overlay as plain entities")?;
    let (mut nd, mut md) = (0u32, 0u8);
    for y in 0..h as usize {
        for x in 0..w as usize {
            if !mask(x, y) {
                continue;
            }
            let k = (y * w as usize + x) * 4;
            let dd = (0..3).map(|j| img[k + j].abs_diff(as_plain[k + j])).max().unwrap_or(0);
            if dd > 2 {
                nd += 1;
            }
            md = md.max(dd);
        }
    }
    dump(&format!("overlay_{level}_gpu"), &img, w, h);
    dump(&format!("overlay_{level}_plain"), &as_plain, w, h);
    t.ok(nd <= 40, &format!("{level}: the overlay (lines, a polyline, a filled triangle, a circle, a window box, a crossing box) against the same geometry as plain entities: {nd} pixels over 2 codes \
                             (largest {md}; {} records, {} styles, {} bytes)", oi.records, oi.styles, oi.bytes));
    let mut reference = refimg::draw(&d, &cam, (w, h), BG);
    // (the reference without the grips' and markers' rows)
    for y in 195..h as usize {
        for x in 0..w as usize {
            let k = (y * w as usize + x) * 4;
            reference[k..k + 4].copy_from_slice(&img[k..k + 4]);
        }
    }
    let tol = Tol { small: 16, over: 400, max: 255, ink: 0.02 };
    let c = refimg::compare(&img, &reference, w, BG, tol.small);
    t.ok(c.fine(&tol), &format!("{level}: the overlay against lib/gfx: {}", c.text()));
    // grips: crisp squares of GRIP_PX on their pixels (the cold one's middle pixel its colour, its outline dark)
    let px = |x: usize, y: usize| {
        let k = (y * w as usize + x) * 4;
        (img[k + 2] as u32) << 16 | (img[k + 1] as u32) << 8 | img[k] as u32
    };
    let cold = px(30, 210);
    let edge = px(26, 210);
    t.ok(cold == render2d::GRIP_COLD & 0xFFFFFF && edge == render2d::GRIP_EDGE & 0xFFFFFF, &format!("{level}: a cold grip: its middle {cold:06x} (want {:06x}), its edge {edge:06x}",
                                                                                                  render2d::GRIP_COLD & 0xFFFFFF));
    // markers: each kind draws some of its colour around its point
    let mut seen = 0;
    for k in 0..9 {
        let cx = 30 + 30 * k;
        let mut n = 0;
        for y in 237..254usize {
            for x in cx - 8..cx + 9 {
                let v = px(x, y);
                if (v >> 16) & 255 > 150 && (v >> 8) & 255 > 100 && v & 255 < 90 {
                    n += 1;
                }
            }
        }
        seen += (n > 10) as u32;
    }
    t.ok(seen == 9, &format!("{level}: the nine snap markers draw their symbols ({seen} of 9)"));
    // a rubber band in drawing units from a point at 1e7 seen at 1e-6 units per pixel: where the camera puts it
    let at = [9_876_543.210_987, -9_999_000.0];
    let mut deep = t.res(Scene::new(&r), "Scene::new")?;
    let dc = Camera { center: at, scale: 1e-6, rotation: 0.4 };
    t.res(deep.set_camera(&dc), "camera")?;
    let a = [at[0] - 1.0e-4, at[1] + 0.3e-4];
    let b = [at[0] + 3.0e6, at[1] + 1.0e6];
    deep.overlay().line(a, b, Space::Drawing, &style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter));
    let img = shot(t, dev, &mut off, &mut r, &mut deep, "a rubber band at 1e7")?;
    dump(&format!("overlay_deep_{level}"), &img, w, h);
    // per column: the line's ink centroid against where to_screen puts the line
    let (pa, pb) = (dc.to_screen(a, (w, h)), dc.to_screen(b, (w, h)));
    let mut worst = 0.0f64;
    let mut cols = 0;
    for x in 20..(w as usize - 20) {
        let (mut sum, mut wsum) = (0.0f64, 0.0f64);
        for y in 0..h as usize {
            let k = (y * w as usize + x) * 4;
            let v = (((BG >> 8) & 255) as f64 - img[k + 1] as f64).max(0.0);
            sum += v * (y as f64 + 0.5);
            wsum += v;
        }
        if wsum < 50.0 {
            continue;
        }
        let xc = x as f64 + 0.5;
        // (the columns at the line's own end hold part of it only)
        if xc < pa[0] + 3.0 {
            continue;
        }
        let tt = (xc - pa[0]) / (pb[0] - pa[0]);
        let yw = pa[1] + tt * (pb[1] - pa[1]);
        // (the line's y at the column's centre; a column where the line leaves the image holds part of it only)
        if !(0.0..=1.0).contains(&tt) || yw < 2.0 || yw > h as f64 - 2.0 {
            continue;
        }
        worst = worst.max((sum / wsum - yw).abs());
        cols += 1;
    }
    t.ok(cols > 50 && worst <= 1.0 / 16.0, &format!("{level}: a rubber band from near 1e7 at 1e-6 units per pixel, turned: each column's centroid within {worst:.4} px of where the camera puts it \
                                                    ({cols} columns; target 1/16)"));
    Some(img)
}

#[cfg(not(target_arch = "wasm32"))]
/// pixels whose colour differs, and the largest difference
pub fn differ(a: &[u8], b: &[u8]) -> (u32, u8) {
    let (mut n, mut m) = (0u32, 0u8);
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let d = (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0);
        if d > 0 {
            n += 1;
            m = m.max(d);
        }
    }
    (n, m)
}

/// (gallery shots, the browser) the highlight test's drawing with its highlights and a text and an insert
pub fn highlight_shot(s: &mut Scene) -> gpu::Result<()> {
    let (d, bits) = hl_drawing();
    let mut layers = Vec::new();
    for &(v, o) in &d.layers {
        let mut l = Layer::new(o);
        l.visible = v;
        layers.push(s.add_layer(&l)?);
    }
    for (k, it) in d.items.iter().enumerate() {
        let st = s.add_style(&it.style)?;
        let id = with_shape(&it.geo, |sh| s.insert(layers[it.layer], st, sh))?;
        s.set_highlight(id, bits[k])?;
    }
    Ok(())
}

/// (gallery shots, the browser) the overlay test's overlay over the joins drawing
pub fn overlay_shot(s: &mut Scene) -> gpu::Result<()> {
    build(s, &joins_drawing(Join::Round, 3.0))?;
    let _ = fill_overlay(s, 260);
    Ok(())
}
