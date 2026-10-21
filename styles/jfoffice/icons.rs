// Office vector artwork rendered with jfguiLib paths and its text engine.
// Scene data uses a resolution-independent 20-unit grid. Motif attribution is in
// LICENSE-icons.txt; the renderer is independent of WordCraft/egui.
use ui::{Gfx, consts::{FONT_REG, FONT_BOLD}};

#[path = "icons_data.rs"]
mod data;

enum Mark {
    Path(&'static [(f32, f32)], bool, bool, u8),
    Box([f32; 4], bool, u8),
    Circle([f32; 3], bool, u8),
    Text([f32; 3], &'static str, bool, u8),
}

fn ink(role: u8, foreground: u32, accent: u32, disabled: bool) -> u32 {
    if disabled { return foreground; }
    match role {
        0 => foreground,
        1 => accent,
        2 => 0xffd13b3b,
        3 => 0xfff2c811,
        4 => 0xff2e8b57,
        5 => 0xffe07b1f,
        6 => 0xffe86fa0,
        7..=11 => {
            let alpha = [46, 77, 102, 128, 153][(role - 7) as usize];
            (accent & 0xffffff) | alpha << 24
        }
        12 => 0x28000000,
        14 => 0x1e000000,
        15 => 0x32000000,
        16 => 0x50000000,
        _ => 0xe6ffffff,
    }
}

fn fixed(v: f32) -> i32 {
    rt::fx::cvt(v * 256.0)
}

/// Draw a registered icon into its physical em box; unknown roles use the SGF1 fallback.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(g: &mut Gfx, cp: u32, x: i32, y: i32, size_fx: i32,
                   foreground: u32, accent: u32, _dark: bool, disabled: bool, dpi: u32) -> bool {
    let Some(scene) = data::scene(cp) else { return false };
    if size_fx <= 0 { return true; }
    let scale = size_fx as f32 / (256.0 * 20.0);
    let ox = x as f32;
    let oy = y as f32;
    let width = fixed(1.25 * scale).max(1);
    let dpi_scale = dpi as f32 / 96.0;
    // Small menu indicators occupy the supplied slot; full-size semantic arrows
    // retain their 20-unit artwork and surrounding whitespace.
    if matches!(cp, ui::keys::IC_CHEV_DOWN | ui::keys::IC_CHEV_DOWN_SMALL)
        && size_fx as f32 <= 9.0 * dpi_scale * 256.0 {
        let side = size_fx as f32 / 256.0;
        let path = &mut g.text.path;
        path.reset();
        path.set_origin(0,0);
        path.move_f(ox,oy+side*0.23);
        path.line_f(ox+side,oy+side*0.23);
        path.line_f(ox+side*0.5,oy+side*0.77);
        path.close();
        g.dc.set_color(foreground);
        path.fill(&mut g.text.ras,g.dc,g.sf,gfx::RULE_NONZERO);
        return true;
    }
    let saved_font = g.text.get_font();
    for mark in scene {
        let role = match *mark {
            Mark::Path(_,_,_,c) | Mark::Box(_,_,c) | Mark::Circle(_,_,c) | Mark::Text(_,_,_,c) => c,
        };
        g.dc.set_color(ink(role, foreground, accent, disabled));
        match mark {
            Mark::Path(points, closed, filled, _) => {
                let path = &mut g.text.path;
                path.reset();
                path.set_origin(0,0);
                if let Some(&(px,py)) = points.first() {
                    path.move_f(ox + px * scale, oy + py * scale);
                    for &(px,py) in &points[1..] {
                        path.line_f(ox + px * scale, oy + py * scale);
                    }
                    if *closed { path.close(); }
                    if *filled {
                        path.fill(&mut g.text.ras,g.dc,g.sf,gfx::RULE_NONZERO);
                    } else {
                        path.stroke(&mut g.text.ras,g.dc,g.sf,width,gfx::CAP_BUTT << 4 | gfx::JOIN_MITER);
                    }
                }
            }
            Mark::Box([left,top,right,bottom], filled, _) => {
                let path = &mut g.text.path;
                path.reset();
                path.set_origin(0,0);
                path.rrect_fx(fixed(ox+left*scale),fixed(oy+top*scale),
                    fixed((right-left)*scale),fixed((bottom-top)*scale),
                    fixed(if *filled { 0.5*dpi_scale } else { dpi_scale }));
                if *filled { path.fill(&mut g.text.ras,g.dc,g.sf,gfx::RULE_NONZERO); }
                else { path.stroke(&mut g.text.ras,g.dc,g.sf,width,gfx::JOIN_MITER); }
            }
            Mark::Circle([cx,cy,radius], filled, _) => {
                let path = &mut g.text.path;
                path.reset();
                path.set_origin(0,0);
                path.ellipse_fx(fixed(ox+cx*scale),fixed(oy+cy*scale),fixed(radius*scale),fixed(radius*scale));
                if *filled { path.fill(&mut g.text.ras,g.dc,g.sf,gfx::RULE_NONZERO); }
                else { path.stroke(&mut g.text.ras,g.dc,g.sf,width,gfx::JOIN_ROUND); }
            }
            Mark::Text([cx,cy,height], value, bold, _) => {
                let size = fixed(height*scale).max(1);
                g.text.set_font(if *bold {FONT_BOLD} else {FONT_REG},size);
                let advance = g.text.measure(value) as f32 / 256.0;
                let tx = rt::fx::cvt(ox+cx*scale-advance*0.5)-g.dc.ox;
                let ty = rt::fx::cvt(oy+cy*scale+(g.text.ascent()-g.text.descent()) as f32*0.5)-g.dc.oy;
                g.text.draw(g.dc,g.sf,tx,ty,value.as_bytes());
            }
        }
    }
    g.text.set_font(saved_font.0,saved_font.1);
    g.dc.set_color(foreground);
    true
}
