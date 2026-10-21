// symbol -- multi-colour vector symbols (device pictures for diagrams and palettes): vicon's
// layers with palette roles and stroke widths, parsed once, drawn as vectors or rasterised
// once per size and palette and blitted
//
//   Symbol::parse(art, vb) -> Symbol     art: SVG path layers separated by '|' on a vb[0] x
//                                        vb[1] grid (tools/vsym.py writes them from SVG files)
//   sym.paint(g, x, y, scale, &pal)      every layer, the grid's (0, 0) at x, y (the draw
//                                        context's coordinates, like Dc::fill), scale = px
//                                        per grid unit
//   sym.raster(g, w, &pal) -> Surface    the symbol w px wide in a PARGB surface
//   sym.height(w) -> px                  the height of the symbol drawn w px wide
//   SymPalette { c: [u32; 10] }          the colours of the roles SYM_INK .. SYM_HI
//   SymPalette::from_theme(ui, tint)     light, dark or high contrast from the theme's tokens;
//                                        tint = the category colour 0xRRGGBB (its light value)
//   SymPalette::light / dark(text, bg, tint), contrast(ui)   the three schemes
//   pal.powered_off()                    LEDs and screens dark (a device without power)
//   SymbolCache::new()                   rasters of up to 256 (symbol, width, palette)
//   cache.draw(g, id, &sym, x, y, w, &pal, opacity)   the symbol w px wide at x, y: the
//                                        cached raster blitted, wider than 256 px the
//                                        vectors; opacity 0..255 (power off: 115 = 45 %)
//   cache.get(g, id, &sym, w, &pal) -> Option<Rc<Surface>>   the raster itself
//
// Layer prefixes (vicon.rs's syntax extended, so its art stays valid): F filled (non-zero
// rule), E filled (even-odd), neither = stroked with round caps and joins; #n palette role
// 0..9 (default 0, an 'A' layer 4); S<w> stroke width in grid units (default 1.5; at least
// 0.8 px when drawn).  Layers paint in order.  Roles: 0 ink (outlines), 1 body, 2 body shade,
// 3 screen, 4 category tint, 5 ports and dark parts, 6 ok, 7 warn, 8 crit (status LEDs),
// 9 highlight.  Light and dark themes swap the palette only.
//
// The cache key is (id, width, palette hash) with the palette compared too; `id` is the
// caller's number for the symbol.  Rasters are drawn at whole pixels, so x, y of draw are
// integers; paint takes fractional positions (a zoomed diagram).

use alloc::rc::Rc;
use alloc::vec::Vec;

use gfx::pathdata::PData;
use gfx::xform::Matrix;
use gfx::{Surface, FMT_PARGB};
use rt::fx::{ceil_i, div255};

use crate::cmd::Gfx;
use crate::consts::*;
use crate::theme::color_mix;
use crate::vicon::layer_head;
use crate::Ui;

pub const SYM_INK: u8 = 0; // outlines
pub const SYM_BODY: u8 = 1; // the device's body
pub const SYM_SHADE: u8 = 2; // a body part in shadow (sides, bezels, recesses)
pub const SYM_SCREEN: u8 = 3; // displays
pub const SYM_TINT: u8 = 4; // the category colour (one band per symbol)
pub const SYM_PORT: u8 = 5; // ports, grilles, dark parts
pub const SYM_OK: u8 = 6; // status LEDs
pub const SYM_WARN: u8 = 7;
pub const SYM_CRIT: u8 = 8;
pub const SYM_HI: u8 = 9; // highlight (light details)
pub const SYM_ROLES: usize = 10;
/// the widest symbol SymbolCache rasterises (px); wider ones are drawn as vectors
pub const SYM_CACHE_MAX_W: i32 = 256;

const CACHE_N: usize = 256;
const STROKE_DEFAULT: f32 = 1.5;
const STROKE_MIN_PX: f32 = 0.8;

struct Layer {
    pd: PData,
    role: u8,
    /// 0 stroked, 1 non-zero fill, 2 even-odd fill
    fill: u8,
    /// stroke width in grid units
    width: f32,
}

/// a parsed multi-colour symbol (see the module comment)
pub struct Symbol {
    layers: Vec<Layer>,
    /// the grid: width and height in grid units
    pub vb: [f32; 2],
}

impl Symbol {
    /// parse the layers once; layers whose path data does not parse are left out
    pub fn parse(art: &str, vb: [f32; 2]) -> Symbol {
        let mut layers = Vec::new();
        for layer in art.split('|') {
            let layer = layer.trim();
            if layer.is_empty() {
                continue;
            }
            let (h, d) = layer_head(layer);
            let mut pd = PData::new();
            if pd.parse(d.as_bytes()).0 != 0 {
                continue;
            }
            let role = h.role.unwrap_or(if h.accent { SYM_TINT } else { SYM_INK });
            layers.push(Layer { pd, role: role.min(SYM_ROLES as u8 - 1), fill: h.fill, width: h.stroke.unwrap_or(STROKE_DEFAULT) });
        }
        let vb = [if vb[0] > 0.0 { vb[0] } else { 1.0 }, if vb[1] > 0.0 { vb[1] } else { 1.0 }];
        Symbol { layers, vb }
    }

    /// the number of layers that parsed
    pub fn layers(&self) -> usize {
        self.layers.len()
    }

    /// the height in px of the symbol drawn w px wide
    pub fn height(&self, w: i32) -> i32 {
        ceil_i(w as f32 * self.vb[1] / self.vb[0]).max(0)
    }

    /// draw every layer with the grid's (0, 0) at x, y (draw context coordinates) and
    /// `scale` px per grid unit; the draw colour is kept
    pub fn paint(&self, g: &mut Gfx, x: f32, y: f32, scale: f32, pal: &SymPalette) {
        if scale <= 0.0 || !scale.is_finite() {
            return;
        }
        let s = scale as f64;
        let m = Matrix { a: s, b: 0.0, c: x as f64 + g.dc.ox as f64, d: 0.0, e: s, f: y as f64 + g.dc.oy as f64 };
        let color = g.dc.get_color();
        for l in &self.layers {
            let path = &mut g.text.path;
            path.reset();
            path.set_origin(0, 0);
            if l.pd.emit(path, Some(&m)) != 0 {
                continue;
            }
            g.dc.set_color(pal.c[l.role as usize]);
            if l.fill != 0 {
                path.fill(&mut g.text.ras, g.dc, g.sf, if l.fill == 2 { gfx::RULE_EVENODD } else { gfx::RULE_NONZERO });
            } else {
                let w = ((l.width * scale).max(STROKE_MIN_PX) * 256.0) as i32;
                path.stroke(&mut g.text.ras, g.dc, g.sf, w, gfx::JOIN_ROUND | gfx::CAP_ROUND << 4);
            }
        }
        g.dc.set_color(color);
    }

    /// the symbol w px wide in a new PARGB surface (height(w) px high)
    pub fn raster(&self, g: &mut Gfx, w: i32, pal: &SymPalette) -> Surface {
        let mut sf = Surface::new(w.max(0), self.height(w), FMT_PARGB);
        if sf.px.is_empty() {
            return sf;
        }
        // begin / end keep the target's clip, origin and opacity
        g.dc.begin(&sf);
        g.dc.set_opacity(255);
        {
            let mut tg = Gfx { dc: &mut *g.dc, sf: &mut sf, text: &mut *g.text };
            self.paint(&mut tg, 0.0, 0.0, w as f32 / self.vb[0], pal);
        }
        g.dc.end();
        sf
    }
}

// fg (straight alpha) over an opaque bg
fn over(fg: u32, bg: u32) -> u32 {
    let a = fg >> 24;
    color_mix(bg | 0xFF00_0000, fg | 0xFF00_0000, ((a * 256 + 127) / 255) as i32)
}

/// the colours of a symbol's roles (straight 0xAARRGGBB, index = SYM_* role)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SymPalette {
    pub c: [u32; SYM_ROLES],
}

impl SymPalette {
    /// the palette of the current theme: high contrast, dark or light; tint = the category
    /// colour 0xRRGGBB as it looks in the light theme
    pub fn from_theme(ui: &Ui, tint: u32) -> SymPalette {
        if ui.high_contrast() {
            return SymPalette::contrast(ui);
        }
        let dark = ui.theme_dark();
        let bg = over(ui.color(T_BG), if dark { 0xFF00_0000 } else { 0xFFFF_FFFF });
        let text = over(ui.color(T_TEXT), bg);
        if dark { SymPalette::dark(text, bg, tint) } else { SymPalette::light(text, bg, tint) }
    }

    /// the light scheme from the text and window colours (opaque): slate outlines, near-white
    /// bodies, pale steel screens
    pub fn light(text: u32, bg: u32, tint: u32) -> SymPalette {
        let ink = color_mix(text, bg, 40);
        let body = color_mix(0xFFFF_FFFF, bg, 96);
        let port = color_mix(ink, body, 40);
        SymPalette {
            c: [
                ink,
                body,
                color_mix(body, ink, 36),
                color_mix(body, 0xFF3D_6A96, 64),
                tint | 0xFF00_0000,
                port,
                0xFF3C_9A5F,
                0xFFD3_9A2C,
                0xFFC8_473D,
                0xFFFF_FFFF,
            ],
        }
    }

    /// the dark scheme: light grey outlines, bodies a step above the window, dark screens,
    /// the tint lighter and greyer
    pub fn dark(text: u32, bg: u32, tint: u32) -> SymPalette {
        let ink = color_mix(text, bg, 64);
        let body = color_mix(bg, 0xFFFF_FFFF, 36);
        SymPalette {
            c: [
                ink,
                body,
                color_mix(body, bg, 128),
                color_mix(bg, 0xFF3D_6A96, 90),
                color_mix(tint | 0xFF00_0000, 0xFFC8_CDD3, 56),
                color_mix(bg, 0xFF00_0000, 64),
                0xFF52_B072,
                0xFFE0_A83E,
                0xFFE0_645A,
                color_mix(body, ink, 128),
            ],
        }
    }

    /// high contrast: outlines and ports in the text colour, every fill in the window colour,
    /// the tint band in the highlight (accent) colour, LEDs in the status colours
    pub fn contrast(ui: &Ui) -> SymPalette {
        let bg = over(ui.color(T_BG), 0xFF00_0000);
        let text = over(ui.color(T_TEXT), bg);
        let tok = |t: u32| over(ui.color(t), bg);
        SymPalette { c: [text, bg, bg, bg, tok(T_ACCENT), text, tok(T_OK), tok(T_WARN), tok(T_CRIT), bg] }
    }

    /// a device without power: status LEDs dark, screens off
    pub fn powered_off(&self) -> SymPalette {
        let mut p = *self;
        let off = color_mix(p.c[SYM_SHADE as usize], p.c[SYM_PORT as usize], 96);
        p.c[SYM_OK as usize] = off;
        p.c[SYM_WARN as usize] = off;
        p.c[SYM_CRIT as usize] = off;
        p.c[SYM_SCREEN as usize] = p.c[SYM_SHADE as usize];
        p
    }

    /// FNV-1a of the colours (the cache key)
    pub fn hash(&self) -> u32 {
        let mut h: u32 = 0x811C_9DC5;
        for c in self.c {
            for b in c.to_le_bytes() {
                h = (h ^ b as u32).wrapping_mul(0x0100_0193);
            }
        }
        h
    }
}

struct Entry {
    id: u32,
    w: i32,
    hash: u32,
    pal: [u32; SYM_ROLES],
    sf: Rc<Surface>,
    used: u32,
}

/// rasters of symbols per (id, width, palette), the least recently used one dropped
/// beyond 256
#[derive(Default)]
pub struct SymbolCache {
    list: Vec<Entry>,
    tick: u32,
}

impl SymbolCache {
    pub fn new() -> SymbolCache {
        SymbolCache::default()
    }

    /// the number of rasters kept
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// drop every raster
    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// the raster of symbol `id` w px wide in this palette, made on the first call; None for
    /// widths outside 1..=SYM_CACHE_MAX_W
    pub fn get(&mut self, g: &mut Gfx, id: u32, sym: &Symbol, w: i32, pal: &SymPalette) -> Option<Rc<Surface>> {
        if w <= 0 || w > SYM_CACHE_MAX_W {
            return None;
        }
        let hash = pal.hash();
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        if let Some(e) = self.list.iter_mut().find(|e| e.id == id && e.w == w && e.hash == hash && e.pal == pal.c) {
            e.used = tick;
            return Some(e.sf.clone());
        }
        let sf = Rc::new(sym.raster(g, w, pal));
        if self.list.len() >= CACHE_N {
            let mut old = 0;
            for (i, e) in self.list.iter().enumerate() {
                if e.used < self.list[old].used {
                    old = i;
                }
            }
            self.list.swap_remove(old);
        }
        self.list.push(Entry { id, w, hash, pal: pal.c, sf: sf.clone(), used: tick });
        Some(sf)
    }

    /// draw symbol `id` w px wide with its top-left corner at x, y (draw context
    /// coordinates) at opacity 0..255: the cached raster, wider than SYM_CACHE_MAX_W the
    /// vectors (through a raster when translucent)
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, g: &mut Gfx, id: u32, sym: &Symbol, x: i32, y: i32, w: i32, pal: &SymPalette, opacity: u32) {
        if w <= 0 || opacity == 0 {
            return;
        }
        let op = opacity.min(255);
        let sf = if w <= SYM_CACHE_MAX_W {
            self.get(g, id, sym, w, pal)
        } else if op == 255 {
            sym.paint(g, x as f32, y as f32, w as f32 / sym.vb[0], pal);
            None
        } else {
            Some(Rc::new(sym.raster(g, w, pal)))
        };
        if let Some(sf) = sf {
            let op0 = g.dc.opac;
            g.dc.set_opacity(div255(op * op0));
            g.dc.blit(g.sf, &sf, x, y);
            g.dc.set_opacity(op0);
        }
    }
}
