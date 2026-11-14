// textgpu -- the text tests of render2d (G6c; docs/RENDER2D.md "Text"): text against lib/gfx's exact fill of the same
// layout's outlines, translucent text blended once, bars, size classes and fills by zoom, text in blocks, the layout's
// alignments, the atlas's bytes; changes, large coordinates and culling of text (textscenes in the same file).
//
//   text        layouts (alignments, MTEXT), images against lib/gfx (sizes, rotation, width factor, oblique angle, mirrors,
//               pixels as unit, serif and sans), translucent, bars, classes S and L, fills, blocks; Voll against Basis
//   textdeep    the same text at the origin and at 1e7; 1e-6 units per pixel with 24 pans of 0.37 px: the ink's centroid within
//               1/16 px of its place; only a glyph's tip or a bar's end in the target: with and without culling the same;
//               changes (add, edit, remove; in blocks) against a scene built anew, and what goes up

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::path::Path;
use gfx::raster::Raster;
use gfx::{Dc, Surface, FMT_XRGB, RULE_NONZERO};
use gpu::*;
use render2d::math::{sincos, sqrt};
use render2d::msdf::Seg;
use render2d::{Camera, EntityId, HAlign, Layer, MText, Renderer, RendererDesc, Scene, Shape, Style, Text, TextLayout, TextStyle, TextStyleId, VAlign};

use crate::gallery::{bg_clear, Aff, Off, BG};
use crate::refimg::{self, Tol};
use crate::tests::{dump, on_levels, out, shot, T};
use crate::gallery::{SANS, SERIF};
use crate::textdraw::*;

/// a point of a text's frame (em) in pixels of the target (y down); xf: the insert it lies in
pub fn em_px(lay: &TextLayout, cam: &Camera, size: (u32, u32), e: [f64; 2], xf: Option<&Aff>) -> [f64; 2] {
    if lay.px {
        let o = cam.to_screen(match xf {
            Some(a) => a.apply(lay.origin),
            None => lay.origin,
        }, size);
        let v = [lay.m[0] * e[0] + lay.m[1] * e[1], lay.m[2] * e[0] + lay.m[3] * e[1]];
        let (s, c) = sincos(cam.rotation);
        return [o[0] + c * v[0] - s * v[1], o[1] - (s * v[0] + c * v[1])];
    }
    let p = lay.to_drawing(e);
    let p = match xf {
        Some(a) => a.apply(p),
        None => p,
    };
    cam.to_screen(p, size)
}

/// a text's glyph outlines as one path in pixels, every piece within 1/256 px
pub fn text_path(fonts: &mut render2d::Fonts, lay: &TextLayout, cam: &Camera, size: (u32, u32), xf: Option<&Aff>, path: &mut Path) {
    for line in &lay.lines {
        for gi in line.glyphs.start..line.glyphs.end {
            let g = lay.glyphs[gi as usize];
            if g.blank {
                continue;
            }
            let Some(sh) = fonts.shape(g.font, g.glyph) else { continue };
            let o = [line.origin[0] + g.pen[0], line.origin[1] + g.pen[1]];
            let px = |p: [f64; 2]| em_px(lay, cam, size, [o[0] + p[0], o[1] + p[1]], xf);
            for c in &sh.contours {
                let mut first = true;
                for s in c {
                    let pts: Vec<[f64; 2]> = match *s {
                        Seg::Line(a, b) => vec![px(a), px(b)],
                        Seg::Quad(a, b, cc) => vec![px(a), px(b), px(cc)],
                        Seg::Cubic(a, b, cc, d) => vec![px(a), px(b), px(cc), px(d)],
                    };
                    if first {
                        path.move_f(pts[0][0] as f32, pts[0][1] as f32);
                        first = false;
                    }
                    let n = match pts.len() {
                        2 => 1,
                        3 => {
                            let d = [pts[0][0] - 2.0 * pts[1][0] + pts[2][0], pts[0][1] - 2.0 * pts[1][1] + pts[2][1]];
                            (sqrt(sqrt(d[0] * d[0] + d[1] * d[1]) * 64.0) as usize + 1).max(1)
                        }
                        _ => {
                            let d1 = [pts[0][0] - 2.0 * pts[1][0] + pts[2][0], pts[0][1] - 2.0 * pts[1][1] + pts[2][1]];
                            let d2 = [pts[1][0] - 2.0 * pts[2][0] + pts[3][0], pts[1][1] - 2.0 * pts[2][1] + pts[3][1]];
                            let m = sqrt(d1[0] * d1[0] + d1[1] * d1[1]).max(sqrt(d2[0] * d2[0] + d2[1] * d2[1]));
                            (sqrt(m * 192.0) as usize + 1).max(1)
                        }
                    };
                    for i in 1..=n {
                        let t = i as f64 / n as f64;
                        let u = 1.0 - t;
                        let q = match pts.len() {
                            2 => pts[1],
                            3 => [u * u * pts[0][0] + 2.0 * u * t * pts[1][0] + t * t * pts[2][0], u * u * pts[0][1] + 2.0 * u * t * pts[1][1] + t * t * pts[2][1]],
                            _ => {
                                let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                                [b0 * pts[0][0] + b1 * pts[1][0] + b2 * pts[2][0] + b3 * pts[3][0], b0 * pts[0][1] + b1 * pts[1][1] + b2 * pts[2][1] + b3 * pts[3][1]]
                            }
                        };
                        path.line_f(q[0] as f32, q[1] as f32);
                    }
                }
                path.close();
            }
        }
    }
}

/// lib/gfx's image of texts (each its own fill, non-zero, exact area, blended once in its colour) on the background
pub fn text_ref(fonts: &mut render2d::Fonts, texts: &[(TextLayout, u32, Option<Aff>)], cam: &Camera, size: (u32, u32), bg: u32) -> Vec<u8> {
    let (w, h) = (size.0 as i32, size.1 as i32);
    let mut sf = Surface::new(w, h, FMT_XRGB);
    let mut dc = Dc::new(w, h);
    dc.begin(&sf);
    dc.clear(&mut sf, bg);
    let mut ras = Raster::default();
    for (lay, color, xf) in texts {
        let mut p = Path::default();
        text_path(fonts, lay, cam, size, xf.as_ref(), &mut p);
        dc.set_color(*color);
        p.fill(&mut ras, &dc, &mut sf, RULE_NONZERO);
    }
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let at = y * sf.stride as usize;
        v.extend_from_slice(&sf.px[at..at + w as usize * 4]);
    }
    v
}

/// the layouts and colours of a built scene's texts
fn refs(s: &Scene, b: &Built, items: &[TItem]) -> Vec<(TextLayout, u32, Option<Aff>)> {
    b.ents.iter().zip(items).filter_map(|(&e, it)| s.text_layout(e).map(|l| (l.clone(), it.color, None))).collect()
}

/// the tolerance of text against lib/gfx: the distance field's edge (corners inside a pixel, thin features at small
/// sizes: docs/RENDER2D.md "Text"): pixels beyond 24 codes per pixel of ink edge, the most, the ink
fn text_tol(img_ref: &[u8], bg: u32, over_share: f64, max: u8, ink: f64) -> Tol {
    // (the reference's partial pixels: the length of its edges in pixels, about)
    let bgc = [(bg & 255) as u8, ((bg >> 8) & 255) as u8, ((bg >> 16) & 255) as u8];
    let mut edge = 0u32;
    for p in img_ref.chunks_exact(4) {
        if (0..3).any(|k| p[k] != bgc[k]) {
            edge += 1;
        }
    }
    Tol { small: 24, over: (edge as f64 * over_share) as u32 + 4, max, ink }
}

/// a drawing on the GPU against lib/gfx; the comparison
#[allow(clippy::too_many_arguments)]
fn against(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, items: &[TItem], cam: &Camera, over_share: f64, max: u8, ink: f64, what: &str) -> Option<(Vec<u8>, refimg::Cmp)> {
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    t.res(s.set_camera(cam), what)?;
    let b = t.res(build(&mut s, items), what)?;
    let img = shot(t, dev, off, r, &mut s, what)?;
    let rf = refs(&s, &b, items);
    let reference = text_ref(s.fonts(), &rf, cam, (off.w, off.h), off.bg);
    let tol = text_tol(&reference, off.bg, over_share, max, ink);
    let c = refimg::compare(&img, &reference, off.w, off.bg, tol.small);
    let name: String = what.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    dump(&format!("{name}_gpu"), &img, off.w, off.h);
    dump(&format!("{name}_ref"), &reference, off.w, off.h);
    t.ok(c.fine(&tol), &format!("{what}: against lib/gfx: {} (tolerance {} beyond 24, {} codes, {:.2} % ink)", c.text(), tol.over, tol.max, 100.0 * tol.ink));
    Some((img, c))
}

// ---------------------------------------------------------------- the test

pub fn text(t: &mut T) {
    layouts(t);
    atlas_bytes(t);
    let mut imgs: Vec<(String, Vec<u8>)> = Vec::new();
    on_levels(t, &mut |t, dev, level| {
        let Some(mut r) = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
        let Some(mut off) = t.res(Off::new(dev, 640, 400), "target") else { return };
        let cam = crate::gallery::pixel_camera(640, 400);
        // (tolerances from the measurement in docs/RENDER2D.md: corners in a pixel, the smallest sizes' thin strokes)
        let cases: [(u32, &str, f64, u8, f64); 5] = [(0, "sizes", 0.05, 192, 0.04), (1, "turned, wide, narrow, oblique, mirrored", 0.04, 192, 0.03), (2, "large: class L, a fill", 0.02, 160, 0.01),
                                                     (3, "MTEXT", 0.05, 192, 0.04), (4, "translucent, bidi, pixels", 0.05, 192, 0.04)];
        for (k, what, share, max, ink) in cases {
            let items = text_drawing(k);
            if let Some((img, _)) = against(t, dev, &mut r, &mut off, &items, &cam, share, max, ink, &format!("{level} text {what}")) {
                imgs.push((format!("{level} {k}"), img));
            }
            if k == 4 {
                // (translucent alone: no pixel blended twice where glyphs or their quads overlap)
                let one = [items[0].clone()];
                if let Some((img, _)) = against(t, dev, &mut r, &mut off, &one, &cam, share, max, ink, &format!("{level} translucent text alone")) {
                    let twice = blended_twice_text(&img, items[0].color);
                    t.ok(twice == 0, &format!("{level} translucent text: {twice} pixels darker than one blend allows"));
                }
            }
        }
        bars_and_classes(t, dev, &mut r, level);
        blocks(t, dev, &mut r, &mut off, level);
    });
    // Voll and Basis
    let voll: Vec<&(String, Vec<u8>)> = imgs.iter().filter(|x| x.0.starts_with("Voll")).collect();
    let basis: Vec<&(String, Vec<u8>)> = imgs.iter().filter(|x| x.0.starts_with("Basis")).collect();
    if !voll.is_empty() {
        let same = voll.iter().zip(&basis).filter(|(a, b)| a.1 == b.1).count();
        t.ok(same == voll.len() && voll.len() == basis.len(), &format!("text: Voll and Basis byte for byte the same: {same} of {} images", voll.len()));
    }
}

/// no channel of a pixel further from the background than one blend of the colour at full coverage
fn blended_twice_text(img: &[u8], color: u32) -> u32 {
    let a = (color >> 24) as f64 / 255.0;
    let col = [(color & 255) as f64, ((color >> 8) & 255) as f64, ((color >> 16) & 255) as f64];
    let bgc = [(BG & 255) as f64, ((BG >> 8) & 255) as f64, ((BG >> 16) & 255) as f64];
    let once: Vec<f64> = (0..3).map(|k| bgc[k] + (col[k] - bgc[k]) * a).collect();
    let mut n = 0;
    for p in img.chunks_exact(4) {
        // (beyond one blend: farther from the background than `once`, in the colour's direction)
        if (0..3).any(|k| {
            let d = col[k] - bgc[k];
            d.abs() > 8.0 && (p[k] as f64 - once[k]) * d.signum() > 2.5
        }) {
            n += 1;
        }
    }
    n
}

/// the layouts' alignment points against where DXF puts them
fn layouts(t: &mut T) {
    let mut fonts = render2d::Fonts::new();
    let (Ok(sans), Ok(_)) = (fonts.add(SANS, 0), fonts.add(SERIF, 0)) else {
        t.ok(false, "text: the fonts load");
        return;
    };
    let styles = [TextStyle::new(sans)];
    let fm = fonts.metrics(sans).unwrap_or(render2d::FontMetrics { units_per_em: 1.0, cap_height: 0.7, x_height: 0.5, ascender: 0.9, descender: -0.2 });
    let h = 10.0;
    let em = h / fm.cap_height;
    let at = [100.0, 50.0];
    let mut worst = 0.0f64;
    let mut bad = Vec::new();
    for ha in [HAlign::Left, HAlign::Center, HAlign::Right, HAlign::Middle] {
        for va in [VAlign::Baseline, VAlign::Bottom, VAlign::Middle, VAlign::Top] {
            let mut tx = Text::new("Haxg", TextStyleId(0), at);
            tx.height = Some(h);
            tx.halign = ha;
            tx.valign = va;
            let Ok(l) = render2d::layout_text(&mut fonts, &styles, &tx) else {
                t.ok(false, "text: layout");
                return;
            };
            let w = l.lines[0].width;
            let ex = match ha {
                HAlign::Center | HAlign::Middle => 0.5 * w,
                HAlign::Right => w,
                _ => 0.0,
            };
            let ey = if ha == HAlign::Middle {
                l.ink.map_or(0.0, |b| 0.5 * (b[1] + b[3]))
            } else {
                match va {
                    VAlign::Bottom => fm.descender,
                    VAlign::Middle => 0.5 * fm.cap_height,
                    VAlign::Top => fm.cap_height,
                    _ => 0.0,
                }
            };
            let p = l.to_drawing([ex, ey]);
            let d = sqrt((p[0] - at[0]) * (p[0] - at[0]) + (p[1] - at[1]) * (p[1] - at[1]));
            worst = worst.max(d);
            if d > 1e-9 {
                bad.push(format!("{ha:?}/{va:?}"));
            }
            // (the em follows the cap height)
            if (sqrt(l.m[1] * l.m[1] + l.m[3] * l.m[3]) - em).abs() > 1e-9 {
                bad.push(format!("{ha:?}/{va:?}: em"));
            }
        }
    }
    t.ok(bad.is_empty(), &format!("text: the alignment point of every horizontal and vertical alignment lands on `at` (worst {worst:.2e} units) {bad:?}"));
    // aligned and fit: the baseline from at to at2, aligned keeps the width factor, fit the height
    let at2 = [180.0, 110.0];
    for ha in [HAlign::Aligned, HAlign::Fit] {
        let mut tx = Text::new("Fit me", TextStyleId(0), at);
        tx.at2 = at2;
        tx.height = Some(h);
        tx.halign = ha;
        let Ok(l) = render2d::layout_text(&mut fonts, &styles, &tx) else { continue };
        let w = l.lines[0].width;
        let p0 = l.to_drawing([0.0, 0.0]);
        let p1 = l.to_drawing([w, 0.0]);
        let d = (p0[0] - at[0]).abs() + (p0[1] - at[1]).abs() + (p1[0] - at2[0]).abs() + (p1[1] - at2[1]).abs();
        let up = sqrt(l.m[1] * l.m[1] + l.m[3] * l.m[3]);
        let height_kept = (up - em).abs() < 1e-9;
        t.ok(d < 1e-9 && height_kept == (ha == HAlign::Fit), &format!("text: {ha:?}: the baseline from at to at2 ({d:.1e}), the height {}", if height_kept { "kept" } else { "follows" }));
    }
    // MTEXT: wrapped lines no wider than the width (but for a word alone), attachments
    let s = "one two three four five six seven eight nine ten eleven twelve";
    let mut m = MText::new(s, TextStyleId(0), at);
    m.height = Some(h);
    m.width = 120.0;
    if let Ok(l) = render2d::layout_mtext(&mut fonts, &styles, &m) {
        let widest = l.lines.iter().map(|x| x.width * em).fold(0.0f64, f64::max);
        let spacing = if l.lines.len() > 1 { (l.lines[0].origin[1] - l.lines[1].origin[1]) * em } else { 0.0 };
        t.ok(l.lines.len() >= 4 && widest <= 120.0 + 1e-9 && (spacing - h * render2d::LINE_SPACING).abs() < 1e-9,
             &format!("MTEXT: {} lines, the widest {widest:.2} of 120 units, baselines {spacing:.4} apart (5/3 of the height)", l.lines.len()));
        // top left: the first line's cap height at `at`
        let p = l.to_drawing([0.0, fm.cap_height]);
        t.ok((p[0] - at[0]).abs() + (p[1] - at[1]).abs() < 1e-9, "MTEXT: top left puts the first line's cap height at `at`");
    }
}

/// the atlas's bytes for a fixed set of texts: their hash (the browser and other targets print the same)
fn atlas_bytes(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        if level != "Basis" {
            return;
        }
        let Some(r) = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
        let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
        let mut items = text_drawing(0);
        items.extend(text_drawing(2));
        if t.res(build(&mut s, &items), "atlas").is_none() {
            return;
        }
        let i = s.text_info();
        out(&format!("  atlas: {} glyphs, {} tiles S, {} tiles L, {} pages, hash {:016x}", i.glyphs, i.tiles_s, i.tiles_l, i.pages, s.atlas_hash()));
        // what is refused: bytes that are no font, a style of a font the scene lacks, a text without a height, a line
        // spacing of 0, an oblique angle beyond 80 degrees, a style the scene lacks
        let Ok(l) = s.add_layer(&Layer::new(9)) else { return };
        let Ok(st) = s.add_style(&Style::new(0xFF00_0000)) else { return };
        let bad_font = s.add_font(b"not a font at all", 0).is_err();
        let bad_style = s.add_text_style(&TextStyle::new(render2d::FontId(99))).is_err();
        let no_height = s.insert(l, st, &Shape::Text(Text::new("x", TextStyleId(0), [0.0, 0.0]))).is_err();
        let mut m = MText::new("x", TextStyleId(0), [0.0, 0.0]);
        m.height = Some(1.0);
        m.line_spacing = 0.0;
        let no_spacing = s.insert(l, st, &Shape::MText(m)).is_err();
        let mut x = Text::new("x", TextStyleId(0), [0.0, 0.0]);
        x.height = Some(1.0);
        x.oblique = Some(1.5);
        let steep = s.insert(l, st, &Shape::Text(x)).is_err();
        let mut y = Text::new("x", TextStyleId(77), [0.0, 0.0]);
        y.height = Some(1.0);
        let no_style = s.insert(l, st, &Shape::Text(y)).is_err();
        t.ok(bad_font && bad_style && no_height && no_spacing && steep && no_style,
             &format!("text: refused: no font {bad_font}, a style without its font {bad_style}, no height {no_height}, line spacing 0 {no_spacing}, oblique 1.5 rad {steep}, no style {no_style}"));
        t.ok(s.atlas_hash() == ATLAS_HASH, &format!("text: the atlas's bytes as on every target (hash {:016x}, expected {ATLAS_HASH:016x})", s.atlas_hash()));
    });
}


/// bars below T_BAR px per em, glyphs above; classes S and L by size; what a zoom sends
fn bars_and_classes(t: &mut T, dev: &Device, r: &mut Renderer, level: &str) {
    let Some(mut off) = t.res(Off::new(dev, 400, 120), "target") else { return };
    let Some(mut s) = t.res(Scene::new(r), "Scene::new") else { return };
    let items = vec![TItem::new("Small text as a bar per word", 1, [10.0, 60.0], 10.0), TItem::new("serif Text, ink tone", 0, [10.0, 20.0], 10.0)];
    let Some(b) = t.res(build(&mut s, &items), "bars") else { return };
    s.set_stats(true);
    // (glyph records: the glyphs with ink; bars: a word each)
    let glyphs: u64 = b.ents.iter().filter_map(|&e| s.text_layout(e)).map(|l| l.glyphs.iter().filter(|g| !g.blank).count() as u64).sum();
    let bars: u64 = b.ents.iter().filter_map(|&e| s.text_layout(e)).map(|l| l.bars.len() as u64).sum();
    // (zoom out until the em is below T_BAR: 10 / cap 0.73 = 13.7 px per em at scale 1)
    let mut seen = Vec::new();
    for (k, scale) in [1.0, 2.5, 4.0].iter().enumerate() {
        let cam = Camera::new([200.0 * scale, 60.0 * scale], *scale);
        if t.res(s.set_camera(&cam), "camera").is_none() {
            return;
        }
        let Some(img) = shot(t, dev, &mut off, r, &mut s, "bars") else { return };
        s.collect_stats();
        let vis = s.take_stats().last().map_or(0, |f| f.visible);
        seen.push(vis);
        dump(&format!("bars_{level}_{k}"), &img, off.w, off.h);
    }
    let info = s.text_info();
    t.ok(seen[0] == glyphs && seen[1] == bars && seen[2] == bars, &format!("{level} text: {glyphs} glyph and {bars} bar records, visible at scale 1 / 2.5 / 4: {seen:?} (bars below {} px per em, 13.7 / 5.5 / 3.4 px here; the scale decided by {:.3})",
                                                                          render2d::TEXT_BAR_BELOW, info.decided_scale));
    // classes: zoom in on a text until the em is beyond TEXT_LARGE_FROM: its glyphs get tiles of class L, only tiles go up
    let before = s.text_info();
    let cam = Camera::new([60.0, 62.0], 0.2);
    if t.res(s.set_camera(&cam), "camera").is_none() {
        return;
    }
    let _ = shot(t, dev, &mut off, r, &mut s, "class L");
    let after = s.text_info();
    let ui = s.upload_info();
    t.ok(after.tiles_l > before.tiles_l && ui.records == 0, &format!("{level} text: zoomed in, class L tiles {} -> {}, {} KB of tiles and {} rows up, {} records", before.tiles_l, after.tiles_l,
                                                                    after.sent_bytes / 1024, after.sent_rows, ui.records));
    // and a fill beyond TEXT_FILL_FROM
    let cam = Camera::new([30.0, 63.0], 0.04);
    if t.res(s.set_camera(&cam), "camera").is_none() {
        return;
    }
    let _ = shot(t, dev, &mut off, r, &mut s, "fill");
    let f = s.text_info();
    t.ok(f.fills >= 1, &format!("{level} text: {} px per em: {} texts drawn as fills", 13.7 / 0.04, f.fills));
}

/// texts in a block, inserted turned, mirrored, scaled unevenly, against lib/gfx through the same transforms
fn blocks(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, level: &str) {
    let Some(mut s) = t.res(Scene::new(r), "Scene::new") else { return };
    let cam = crate::gallery::pixel_camera(640, 400);
    if t.res(s.set_camera(&cam), "camera").is_none() {
        return;
    }
    let Some(fonts) = t.res(add_fonts(&mut s), "fonts") else { return };
    let Some(layer) = t.res(s.add_layer(&Layer::new(0)), "layer") else { return };
    let Some(blk) = t.res(s.add_block(&render2d::Block::new()), "block") else { return };
    let it = TItem::new("Block AVg", 1, [0.0, 0.0], 12.0);
    let Some(e) = t.res(add_text(&mut s, &fonts, None, Some(blk), &it), "text in a block") else { return };
    let Some(st) = t.res(s.add_style(&Style::new(0xFF30_3030)), "style") else { return };
    let ins = [crate::gallery::BIns::new(0, [40.0, 340.0]), crate::gallery::BIns::new(0, [300.0, 330.0]).turned(0.6, [1.5, 1.5]), crate::gallery::BIns::new(0, [600.0, 250.0]).turned(0.0, [-1.0, 1.0]),
               crate::gallery::BIns::new(0, [60.0, 120.0]).turned(0.3, [2.5, 0.8]), crate::gallery::BIns::new(0, [400.0, 80.0]).turned(-0.4, [1.2, -1.6])];
    for b in &ins {
        let mut i = render2d::Insert::new(blk, b.at);
        i.rotation = b.rotation;
        i.scale = b.scale;
        if t.res(s.insert(layer, st, &Shape::Insert(i)), "insert").is_none() {
            return;
        }
    }
    let Some(img) = shot(t, dev, off, r, &mut s, "blocks") else { return };
    let Some(lay) = s.text_layout(e).cloned() else { return };
    let rf: Vec<(TextLayout, u32, Option<Aff>)> = ins.iter().flat_map(|b| b.xfs([0.0, 0.0])).map(|a| (lay.clone(), it.color, Some(a))).collect();
    let reference = text_ref(s.fonts(), &rf, &cam, (off.w, off.h), off.bg);
    let tol = text_tol(&reference, off.bg, 0.05, 192, 0.04);
    let c = refimg::compare(&img, &reference, off.w, off.bg, tol.small);
    dump(&format!("text_blocks_{level}_gpu"), &img, off.w, off.h);
    dump(&format!("text_blocks_{level}_ref"), &reference, off.w, off.h);
    t.ok(c.fine(&tol), &format!("{level} text in a block, inserted turned, scaled, mirrored, unevenly: against lib/gfx: {}", c.text()));
}

// ---------------------------------------------------------------- large coordinates, culling, changes

/// the items moved by d (drawing units)
fn moved(items: &[TItem], d: [f64; 2]) -> Vec<TItem> {
    items.iter().map(|it| {
        let mut x = it.clone();
        x.at = [it.at[0] + d[0], it.at[1] + d[1]];
        x.at2 = [it.at2[0] + d[0], it.at2[1] + d[1]];
        x
    }).collect()
}

/// the ink's centroid of an image against the background (x, y px) and its weight
fn centroid(img: &[u8], w: u32, bg: u32) -> ([f64; 2], f64) {
    let b = [(bg & 255) as f64, ((bg >> 8) & 255) as f64, ((bg >> 16) & 255) as f64];
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for (i, p) in img.chunks_exact(4).enumerate() {
        let a = (0..3).map(|k| (p[k] as f64 - b[k]).abs()).fold(0.0, f64::max);
        if a > 0.0 {
            let (x, y) = ((i as u32 % w) as f64 + 0.5, (i as u32 / w) as f64 + 0.5);
            sx += a * x;
            sy += a * y;
            sw += a;
        }
    }
    ([sx / sw.max(1e-9), sy / sw.max(1e-9)], sw)
}

pub fn textdeep(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let Some(mut r) = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
        far(t, dev, &mut r, level);
        deep(t, dev, &mut r, level);
        culled(t, dev, &mut r, level);
        changes(t, dev, &mut r, level);
    });
}

/// the same texts at the origin and at three places up to 1e7 away: the same image
fn far(t: &mut T, dev: &Device, r: &mut Renderer, level: &str) {
    let Some(mut off) = t.res(Off::new(dev, 640, 400), "target") else { return };
    let mut items = text_drawing(1);
    items.extend(text_drawing(4).into_iter().take(1));
    let image = |t: &mut T, r: &mut Renderer, off: &mut Off, d: [f64; 2]| -> Option<Vec<u8>> {
        let mut s = t.res(Scene::new(r), "Scene::new")?;
        let mut cam = crate::gallery::pixel_camera(640, 400);
        cam.center = [cam.center[0] + d[0], cam.center[1] + d[1]];
        t.res(s.set_camera(&cam), "camera")?;
        t.res(build(&mut s, &moved(&items, d)), "far")?;
        shot(t, dev, off, r, &mut s, "far")
    };
    let Some(home) = image(t, r, &mut off, [0.0, 0.0]) else { return };
    let mut worst = (0u32, 0u8);
    for d in [[1e7, 1e7], [-9_999_999.5, core::f64::consts::PI * 1e6], [9_876_543.210_987, -9_999_000.0]] {
        let Some(img) = image(t, r, &mut off, d) else { return };
        let x = crate::gallery::differ(&home, &img);
        if x.0 > worst.0 {
            worst = x;
        }
    }
    t.ok(worst.0 == 0, &format!("{level} text at (1e7, 1e7), (-9 999 999.5, pi 1e6), (9 876 543.210987, -9 999 000) against at the origin: {} pixels differ, by up to {}", worst.0, worst.1));
}

/// 1e-6 units per pixel near 1e7: texts (glyphs, and one large enough for its fill) through 24 pans of 0.37 px and a
/// rotation: the ink's centroid against lib/gfx's at the same camera
fn deep(t: &mut T, dev: &Device, r: &mut Renderer, level: &str) {
    let (w, h) = (320u32, 200u32);
    let Some(mut off) = t.res(Off::new(dev, w, h), "target") else { return };
    let at = [-9_876_543.21, 6_543_210.987];
    let k = 1e-6;
    let mut a = TItem::new("Deep AVg 1e-6", 1, [at[0] - 140.0 * k, at[1] + 20.0 * k], 18.0 * k);
    a.rotation = 0.1;
    let mut b = TItem::new("Wg", 0, [at[0] - 60.0 * k, at[1] - 90.0 * k], 120.0 * k);
    b.width = 0.8;
    let items = vec![a, b];
    let Some(mut s) = t.res(Scene::new(r), "Scene::new") else { return };
    if t.res(s.set_camera(&Camera::new(at, k)), "camera").is_none() {
        return;
    }
    let Some(built) = t.res(build(&mut s, &items), "deep") else { return };
    let mut worst = 0.0f64;
    let mut fills = 0;
    for step in 0..24u32 {
        let mut cam = Camera::new([at[0] + step as f64 * 0.37 * k, at[1] - step as f64 * 0.23 * k], k);
        cam.rotation = if step >= 16 { 0.25 } else { 0.0 };
        if t.res(s.set_camera(&cam), "camera").is_none() {
            return;
        }
        let Some(img) = shot(t, dev, &mut off, r, &mut s, "deep") else { return };
        fills = fills.max(s.text_info().fills);
        // each text alone: its centroid against the reference's
        let rf = refs(&s, &built, &items);
        let reference = text_ref(s.fonts(), &rf, &cam, (w, h), off.bg);
        let (cg, _) = centroid(&img, w, off.bg);
        let (cr, _) = centroid(&reference, w, off.bg);
        let d = sqrt((cg[0] - cr[0]) * (cg[0] - cr[0]) + (cg[1] - cr[1]) * (cg[1] - cr[1]));
        worst = worst.max(d);
        if step == 0 {
            dump(&format!("text_deep_{level}_gpu"), &img, w, h);
            dump(&format!("text_deep_{level}_ref"), &reference, w, h);
        }
    }
    t.ok(worst <= 1.0 / 16.0 && fills == 1, &format!("{level} text at 1e-6 units per pixel near ({}, {}), 24 pans of 0.37 px and a rotation: the ink's centroid at most {worst:.4} px from lib/gfx's \
                                                     (1/16 = 0.0625; {fills} of the two as a fill)", at[0], at[1]));
}

/// only a glyph's tip, a bar's end, an insert's glyph at the target's edge: with and without culling the same image
fn culled(t: &mut T, dev: &Device, r: &mut Renderer, level: &str) {
    let (w, h) = (200u32, 100u32);
    let Some(mut off) = t.res(Off::new(dev, w, h), "target") else { return };
    // (the camera: units are pixels, (0, 0) the target's lower left)
    let cases: [(&str, f64); 3] = [("a glyph's right end 1.5 px inside the left edge", 1.0), ("a bar's end 1 px inside the left edge", 4.0), ("a turned text's lowest corner 1 px below the top edge", 1.0)];
    for (k, &(what, scale)) in cases.iter().enumerate() {
        let mut it = TItem::new("Tip W", 1, [0.0, 0.0], 20.0);
        if k == 2 {
            it.rotation = 0.5;
        }
        // (the text's quad where it lies at the origin, then moved so that only that end comes into view)
        let Some(mut probe) = t.res(Scene::new(r), "Scene::new") else { return };
        let Some(b) = t.res(build(&mut probe, &[it.clone()]), "culled") else { return };
        let Some(lay) = probe.text_layout(b.ents[0]).cloned() else { return };
        // (the ink's extreme points: its outlines' points on the curve, in the drawing)
        let (mut right, mut low) = (f64::MIN, f64::MAX);
        for line in &lay.lines {
            for gi in line.glyphs.start..line.glyphs.end {
                let g = lay.glyphs[gi as usize];
                let Some(sh) = probe.fonts().shape(g.font, g.glyph).cloned() else { continue };
                for c in &sh.contours {
                    for sg in c {
                        let p = sg.start();
                        let d = lay.to_drawing([line.origin[0] + g.pen[0] + p[0], line.origin[1] + g.pen[1] + p[1]]);
                        right = right.max(d[0]);
                        low = low.min(d[1]);
                    }
                }
            }
        }
        it.at = match k {
            0 => [1.5 - right, 50.0 * scale],
            1 => [scale - right, 50.0 * scale],
            _ => [60.0, h as f64 - 1.0 - low],
        };
        let cam = Camera::new([0.5 * w as f64 * scale, 0.5 * h as f64 * scale], scale);
        let mut imgs = Vec::new();
        for culling in [true, false] {
            let Some(mut s) = t.res(Scene::new(r), "Scene::new") else { return };
            s.set_culling(culling);
            if t.res(s.set_camera(&cam), "camera").is_none() {
                return;
            }
            if t.res(build(&mut s, &[it.clone()]), "culled").is_none() {
                return;
            }
            let Some(img) = shot(t, dev, &mut off, r, &mut s, "culled") else { return };
            imgs.push(img);
        }
        let ink_px = imgs[1].chunks_exact(4).filter(|p| p[0] as u32 != (BG & 255)).count();
        let (n, m) = crate::gallery::differ(&imgs[0], &imgs[1]);
        t.ok(n == 0 && ink_px > 0, &format!("{level} text, {what}: culled and not culled {n} pixels apart (by {m}); {ink_px} pixels of ink in view"));
    }
}

/// texts added, edited, removed, a block's text changed: each frame the same image as a scene built anew; what goes up
fn changes(t: &mut T, dev: &Device, r: &mut Renderer, level: &str) {
    let (w, h) = (480u32, 240u32);
    let Some(mut off) = t.res(Off::new(dev, w, h), "target") else { return };
    let cam = crate::gallery::pixel_camera(w, h);
    let mut items = vec![TItem::new("First text", 1, [10.0, 200.0], 14.0), TItem::new("Second one", 0, [10.0, 160.0], 14.0), TItem::new("Third, turned", 1, [200.0, 120.0], 12.0)];
    items[2].rotation = 0.3;
    let block_text = |s: &str| TItem::new(s, 1, [0.0, 0.0], 10.0);
    // (a scene: the items on a layer, a block with a text inserted twice)
    let make = |t: &mut T, r: &mut Renderer, items: &[TItem], bt: &str| -> Option<(Scene, Built, EntityId)> {
        let mut s = t.res(Scene::new(r), "Scene::new")?;
        t.res(s.set_camera(&cam), "camera")?;
        let b = t.res(build(&mut s, items), "changes")?;
        let blk = t.res(s.add_block(&render2d::Block::new()), "block")?;
        let e = t.res(add_text(&mut s, &b.fonts, None, Some(blk), &block_text(bt)), "block text")?;
        let st = t.res(s.add_style(&Style::new(0xFF30_5030)), "style")?;
        for (p, rot) in [([300.0, 40.0], 0.0), ([400.0, 200.0], -0.5)] {
            let mut i = render2d::Insert::new(blk, p);
            i.rotation = rot;
            t.res(s.insert(b.layer, st, &Shape::Insert(i)), "insert")?;
        }
        Some((s, b, e))
    };
    let Some((mut s, b, be)) = make(t, r, &items, "In a block") else { return };
    if shot(t, dev, &mut off, r, &mut s, "changes").is_none() {
        return;
    }
    let mut ents = b.ents.clone();
    let mut bt = String::from("In a block");
    let mut fine = 0;
    let mut notes = Vec::new();
    for round in 0..5 {
        match round {
            // the second text edited, the same glyphs: its records only
            0 => {
                items[1].s = "Second nee".into();
                let it = items[1].clone();
                let ts = s.add_text_style(&TextStyle::new(b.fonts[it.font]).with_fallbacks(&[b.fonts[2]])).ok();
                if let Some(ts) = ts {
                    let _ = with_text(&it, ts, |sh| s.update(ents[1], sh));
                }
            }
            // a text with new glyphs added
            1 => {
                let it = TItem::new("Quixotic zebra", 0, [200.0, 60.0], 16.0);
                items.push(it.clone());
                if let Some(e) = t.res(add_text(&mut s, &b.fonts, Some(b.layer), None, &it), "add") {
                    ents.push(e);
                }
            }
            // the first removed
            2 => {
                let _ = s.remove(ents[0]);
                items.remove(0);
                ents.remove(0);
            }
            // the block's text changed (once for both inserts)
            3 => {
                bt = "Block edited".into();
                let it = block_text(&bt);
                if let Ok(ts) = s.add_text_style(&TextStyle::new(b.fonts[1]).with_fallbacks(&[b.fonts[2]])) {
                    let _ = with_text(&it, ts, |sh| s.update(be, sh));
                }
            }
            // a translucent colour (a style): the records of that text
            _ => {
                if let Ok(st) = s.add_style(&Style::new(0x9020_40C0)) {
                    let _ = s.set_entity_style(ents[0], st);
                    items[0].color = 0x9020_40C0;
                }
            }
        }
        let Some(img) = shot(t, dev, &mut off, r, &mut s, "changes") else { return };
        let ui = s.upload_info();
        let ti = s.text_info();
        notes.push(format!("{} records, {} KB tiles", ui.records, ti.sent_bytes / 1024));
        let Some((mut fresh, _, _)) = make(t, r, &items, &bt) else { return };
        let Some(want) = shot(t, dev, &mut off, r, &mut fresh, "changes, built anew") else { return };
        let (n, m) = crate::gallery::differ(&img, &want);
        if n == 0 {
            fine += 1;
        } else {
            t.ok(false, &format!("{level} text changes, round {round}: {n} pixels differ from a scene built anew (by {m})"));
        }
    }
    t.ok(fine == 5, &format!("{level} text changes (edited, added with new glyphs, removed, a block's text, a style): {fine} of 5 frames as a scene built anew; sent: {}", notes.join(" / ")));
}

/// --text-shot DIR: the drawings of the text test on the device's level as DIR/text_K.bgra and their references
pub fn text_shot(dir: &str) -> i32 {
    let mut t = T { fails: 0, validate: false };
    let mut first = true;
    on_levels(&mut t, &mut |t, dev, level| {
        if !first {
            return;
        }
        first = false;
        let Some(mut r) = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
        let Some(mut off) = t.res(Off::new(dev, 640, 400), "target") else { return };
        let cam = crate::gallery::pixel_camera(640, 400);
        for k in 0..5 {
            let items = text_drawing(k);
            let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
            let _ = s.set_camera(&cam);
            let Some(b) = t.res(build(&mut s, &items), "build") else { return };
            let Some(img) = shot(t, dev, &mut off, &mut r, &mut s, "shot") else { return };
            let rf = refs(&s, &b, &items);
            let reference = text_ref(s.fonts(), &rf, &cam, (off.w, off.h), off.bg);
            for (name, im) in [("gpu", &img), ("ref", &reference)] {
                let mut v = Vec::new();
                v.extend_from_slice(&off.w.to_le_bytes());
                v.extend_from_slice(&off.h.to_le_bytes());
                v.extend_from_slice(im);
                let _ = plat::write_file(&format!("{dir}/text_{k}_{name}.bgra"), &v);
            }
            out(&format!("{level} {k}: {}", s.text_info().glyphs));
        }
        // the look of small text: the same lines at 12, 9 and 7.5 px per em (glyphs), 6.5, 5 and 3.5 (bars), sans and serif
        let Some(mut off) = t.res(Off::new(dev, 300, 150), "target") else { return };
        let mut items = Vec::new();
        for (k, em) in [12.0, 9.0, 7.5, 6.5, 5.0, 3.5].iter().enumerate() {
            for (f, cap, x) in [(1usize, 0.727, 6.0), (0, 0.729, 152.0)] {
                items.push(TItem::new("Flst. 1234/5 Wohnhaus", f, [x, 140.0 - 23.0 * k as f64], em * cap));
            }
        }
        let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
        let _ = s.set_camera(&crate::gallery::pixel_camera(300, 150));
        if t.res(build(&mut s, &items), "bars").is_none() {
            return;
        }
        let Some(img) = shot(t, dev, &mut off, &mut r, &mut s, "bars") else { return };
        let mut v = Vec::new();
        v.extend_from_slice(&300u32.to_le_bytes());
        v.extend_from_slice(&150u32.to_le_bytes());
        v.extend_from_slice(&img);
        let _ = plat::write_file(&format!("{dir}/text_bars.bgra"), &v);
        let _ = bg_clear(BG);
    });
    if t.fails > 0 { 1 } else { 0 }
}
