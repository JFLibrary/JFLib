// Numerical acceptance of the shared font and fractional-panel paths.
use alloc::{format, vec::Vec};
use gfx::{Dc, Surface, FMT_A8, FMT_PARGB};
use gfx::{path::Path, raster::Raster, shapes::Shapes};
use text::{FontRaster, GcRef, Text};
use ui::Style;

fn mask(t: &mut Text, id: u32) -> Vec<u8> {
    let glyph = t.glyph(id, 'g' as u32);
    match t.gc_get(id, glyph, 23 * 256, 2) {
        GcRef::Entry(e) => e.px.clone(), _ => Vec::new(),
    }
}

fn panel(sf: &mut Surface, dc: &mut Dc, r: [i32; 4], radius: i32, width: i32, fill: u32, stroke: u32) {
    let mut path = Path::default();
    let mut ras = Raster::default();
    Shapes { sf, dc, path: &mut path, ras: &mut ras }.panel_fx(r[0], r[1], r[2], r[3], radius, width, fill, stroke);
}

pub(super) fn run() -> i32 {
    let mut passed = 0;
    let mut failed = 0;
    macro_rules! check { ($condition:expr, $name:expr) => {
        if $condition { passed += 1; } else { failed += 1; plat::write_stdout(format!("FAIL rendering: {}\n", $name).as_bytes()); }
    }; }
    let mut t = Text::new(style::STYLE.fonts(), style::STYLE.font_names());
    check!((0..4).all(|id| t.valid(id)), "mixed TrueType/SGF embedded slots");
    let a = t.glyph(0, 'A' as u32);
    let v = t.glyph(0, 'V' as u32);
    check!(t.kern(0, a, v) < 0 && t.kern_table(0, a, v) == 0, "GPOS spacing survives embedding without legacy kern table");
    t.set_font(0, 23 * 256);
    let before = t.measure("AV Äöü Straße");
    let caret = t.caret_x("Äöü Straße".as_bytes(), 6);
    let legacy = mask(&mut t, 0);
    check!(t.set_font_raster(0, FontRaster::Natural), "set valid face");
    let natural = mask(&mut t, 0);
    check!(!natural.is_empty() && natural != legacy, "cached glyphs invalidate on raster change");
    check!(t.measure("AV Äöü Straße") == before && t.caret_x("Äöü Straße".as_bytes(), 6) == caret, "raster policy preserves advances and UTF8 caret");
    check!(t.font_raster(1) == FontRaster::Legacy && t.font_raster(3) == FontRaster::Legacy, "other faces remain independent");
    check!(!t.set_font_raster(u32::MAX, FontRaster::Natural), "invalid id does not alter fallback");
    t.set_font_raster(0, FontRaster::Legacy);
    check!(mask(&mut t, 0) == legacy, "legacy cache rebuild is byte exact");
    let runtime = t.load_mem(super::typography::FONTS[3], 0) as u32;
    check!(t.font_raster(runtime) == FontRaster::Legacy, "runtime document font retains legacy raster");
    t.set_font_raster(runtime, FontRaster::Natural);
    let syn = t.style(runtime, text::FS_SYN_ITALIC) as u32;
    check!(t.font_raster(syn) == FontRaster::Natural, "derived face inherits raster");
    t.unload(runtime);
    let replacement = t.load_mem(super::typography::FONTS[3], 0) as u32;
    check!(replacement == runtime && t.font_raster(replacement) == FontRaster::Legacy, "reused slot resets raster");
    let mut p = Path::default();
    check!(p.set_tolerance(0.01) == 0.2 && p.set_tolerance(f32::NAN) == 0.01 && p.set_tolerance(0.2) == 0.01, "bounded curve tolerance and invalid value");

    let mut sf = Surface::new(12, 12, FMT_PARGB);
    let mut dc = Dc::new(12, 12);
    panel(&mut sf, &mut dc, [640, 640, 1024, 1024], 0, 0, 0xffffffff, 0);
    let alpha = |s: &Surface, x: usize, y: usize| s.px[y * s.stride as usize + x * 4 + 3];
    check!(alpha(&sf, 2, 2) == 64 && alpha(&sf, 3, 2) == 128 && alpha(&sf, 3, 3) == 255, "quarter corner and half-edge coverage");
    check!(alpha(&sf, 1, 2) == 0 && alpha(&sf, 7, 2) == 0, "fractional shape bounds");
    let r = [640, 640, 1792, 1792];
    let mut fill = Surface::new(12, 12, FMT_PARGB);
    let mut ring = Surface::new(12, 12, FMT_PARGB);
    panel(&mut fill, &mut dc, r, 700, 0, 0xffa0c0e0, 0);
    panel(&mut ring, &mut dc, r, 700, 256, 0xffa0c0e0, 0xffa0c0e0);
    check!(fill.px == ring.px, "same-color fill and stroke have no compositing seam");
    let mut clipped = Surface::new(12, 12, FMT_PARGB);
    dc.clip_push(4, 4, 3, 3);
    panel(&mut clipped, &mut dc, r, 700, 256, 0xffa0c0e0, 0xffa0c0e0);
    dc.clip_pop();
    check!((0..12).all(|y| (0..12).all(|x| alpha(&clipped,x,y) == if (4..7).contains(&x) && (4..7).contains(&y) { alpha(&fill,x,y) } else { 0 })), "dirty clipping matches full rendering");
    let mut moved = Surface::new(12, 12, FMT_PARGB);
    dc.origin(2, -1);
    panel(&mut moved, &mut dc, [r[0]-512,r[1]+256,r[2],r[3]], 700, 0, 0xffa0c0e0, 0);
    dc.origin(0, 0);
    check!(moved.px == fill.px, "draw origin preserves fractional coverage");
    // A partially repainted tall panel starts its mask bands at a different row.
    // It must nevertheless have identical edges and premultiplied colors.
    let mut tall = Surface::new(36, 170, FMT_PARGB);
    let mut part = Surface::new(36, 170, FMT_PARGB);
    let mut tall_dc = Dc::new(36, 170);
    let tall_rect = [384, 448, 8192, 41728];
    panel(&mut tall, &mut tall_dc, tall_rect, 3200, 384, 0xc0a0c0e0, 0x90605040);
    tall_dc.clip_push(0, 19, 36, 137);
    panel(&mut part, &mut tall_dc, tall_rect, 3200, 384, 0xc0a0c0e0, 0x90605040);
    check!((19..156).all(|y| tall.px[y*tall.stride as usize..(y+1)*tall.stride as usize]
        == part.px[y*part.stride as usize..(y+1)*part.stride as usize]), "mask bands stay seamless under partial repaint");
    check!(tall.px.chunks_exact(4).all(|p| p[..3].iter().all(|v| *v <= p[3])), "translucent panel stays premultiplied");
    let mut a8 = Surface::new(12, 12, FMT_A8);
    panel(&mut a8, &mut dc, r, 700, 256, 0xffa0c0e0, 0xffa0c0e0);
    check!((0..12).all(|y| (0..12).all(|x| a8.px[y*a8.stride as usize+x] == alpha(&fill,x,y))), "A8 equals PARGB alpha");
    let mut opacity = Surface::new(12, 12, FMT_PARGB);
    dc.set_opacity(128);
    panel(&mut opacity, &mut dc, [0,0,3072,3072], 0, 0, 0xffffffff, 0);
    check!(alpha(&opacity, 5, 5) == 128, "global opacity");
    dc.set_opacity(255);
    let saved = sf.px.clone();
    panel(&mut sf, &mut dc, [i32::MAX,0,256,256], 0, 0, 0xffffffff, 0);
    panel(&mut sf, &mut dc, [0,0,-256,256], 0, 0, 0xffffffff, 0);
    dc.origin(i32::MAX,i32::MIN);
    panel(&mut sf, &mut dc, [0,0,256,256], 0, 0, 0xffffffff, 0);
    check!(sf.px == saved, "invalid/overflow/offscreen geometry is a no-op");
    plat::write_stdout(format!("Office rendering: {passed} passed, {failed} failed\n").as_bytes());
    if failed == 0 { 0 } else { 1 }
}
