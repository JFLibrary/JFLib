// textdraw -- the text drawings both paths draw (G6c): the tests on Vulkan (textgpu.rs, --dump) and the browser (web.rs,
// tests/engine_web.mjs compare their images); a text of a drawing, the scene's fonts, the drawings, the atlas's hash

use alloc::string::String;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Attach, EntityId, FontId, HAlign, Layer, MText, Scene, Shape, Style, Text, TextStyle, TextStyleId, Unit, VAlign};

use crate::gallery::{MONO, SANS, SERIF};

/// a text of a test drawing: one line or several, its font (0 serif, 1 sans, 2 mono), its style's width factor, oblique
/// angle and unit, its colour
#[derive(Clone, Debug)]
pub struct TItem {
    pub color: u32,
    pub font: usize,
    pub width: f64,
    pub oblique: f64,
    pub unit: Unit,
    pub s: String,
    pub at: [f64; 2],
    pub at2: [f64; 2],
    pub height: f64,
    pub rotation: f64,
    pub halign: HAlign,
    pub valign: VAlign,
    pub backward: bool,
    pub upside_down: bool,
    /// MTEXT: wrap width (Some: several lines), attachment, spacing
    pub multi: Option<(f64, Attach, f64)>,
}

impl TItem {
    pub fn new(s: &str, font: usize, at: [f64; 2], height: f64) -> TItem {
        TItem { color: 0xFF20_2830, font, width: 1.0, oblique: 0.0, unit: Unit::Drawing, s: s.into(), at, at2: at, height, rotation: 0.0, halign: HAlign::Left, valign: VAlign::Baseline,
                backward: false, upside_down: false, multi: None }
    }
}

/// the scene's three fonts
pub fn add_fonts(s: &mut Scene) -> Result<[FontId; 3]> {
    Ok([s.add_font(SERIF, 0)?, s.add_font(SANS, 0)?, s.add_font(MONO, 0)?])
}

/// a text into a scene: its style, its entity on a layer (or in a block)
pub fn add_text(s: &mut Scene, fonts: &[FontId; 3], layer: Option<render2d::LayerId>, block: Option<render2d::BlockId>, it: &TItem) -> Result<EntityId> {
    let mut ts = TextStyle::new(fonts[it.font]).with_fallbacks(&[fonts[2]]);
    ts.width = it.width;
    ts.oblique = it.oblique;
    ts.unit = it.unit;
    let tsid = s.add_text_style(&ts)?;
    let st = s.add_style(&Style::new(it.color))?;
    with_text(it, tsid, |sh| match (layer, block) {
        (_, Some(b)) => s.insert_in(b, st, sh),
        (Some(l), None) => s.insert(l, st, sh),
        _ => Err(GpuError::Invalid("add_text: a layer or a block")),
    })
}

/// f with the item's shape
pub fn with_text<R>(it: &TItem, ts: TextStyleId, f: impl FnOnce(&Shape) -> R) -> R {
    match it.multi {
        Some((w, a, ls)) => {
            let mut m = MText::new(&it.s, ts, it.at);
            m.height = Some(it.height);
            m.width = w;
            m.rotation = it.rotation;
            m.attach = a;
            m.line_spacing = ls;
            f(&Shape::MText(m))
        }
        None => {
            let mut t = Text::new(&it.s, ts, it.at);
            t.at2 = it.at2;
            t.height = Some(it.height);
            t.rotation = it.rotation;
            t.halign = it.halign;
            t.valign = it.valign;
            t.backward = it.backward;
            t.upside_down = it.upside_down;
            f(&Shape::Text(t))
        }
    }
}

/// a scene of texts on one layer: their entities and layouts
#[allow(dead_code)]
pub struct Built {
    pub ents: Vec<EntityId>,
    pub layer: render2d::LayerId,
    pub fonts: [FontId; 3],
}

pub fn build(s: &mut Scene, items: &[TItem]) -> Result<Built> {
    let fonts = add_fonts(s)?;
    let layer = s.add_layer(&Layer::new(0))?;
    let mut ents = Vec::new();
    for it in items {
        ents.push(add_text(s, &fonts, Some(layer), None, it)?);
    }
    Ok(Built { ents, layer, fonts })
}

// ---------------------------------------------------------------- drawings

const PANGRAM: &str = "Sphinx of black quartz, judge my vow 0123";

/// texts of every size, turn and look in a 640 x 400 target (one unit a pixel, y up from the bottom)
pub fn text_drawing(kind: u32) -> Vec<TItem> {
    let mut v = Vec::new();
    match kind {
        // sizes: cap heights 7 to 40 px, sans and serif
        0 => {
            let mut y = 380.0;
            for (k, h) in [7.0, 9.0, 12.0, 16.0, 22.0, 30.0, 40.0].iter().enumerate() {
                y -= h * 1.5 + 4.0;
                v.push(TItem::new(PANGRAM, k % 2, [8.3, y + 0.37], *h));
            }
        }
        // turned, wider, narrower, oblique, mirrored
        1 => {
            let mut a = TItem::new("Rotated 30\u{b0} AVA", 1, [40.0, 40.0], 18.0);
            a.rotation = 0.5236;
            v.push(a);
            let mut b = TItem::new("Wide 1.6 Wg", 0, [300.0, 330.0], 16.0);
            b.width = 1.6;
            v.push(b);
            let mut c = TItem::new("Narrow 0.5 Wg", 0, [300.0, 290.0], 16.0);
            c.width = 0.5;
            v.push(c);
            let mut d = TItem::new("Oblique 15\u{b0} Rg", 1, [300.0, 250.0], 16.0);
            d.oblique = 0.2618;
            v.push(d);
            let mut e = TItem::new("Backward", 1, [600.0, 200.0], 16.0);
            e.backward = true;
            v.push(e);
            let mut f = TItem::new("Upside down", 1, [300.0, 120.0], 16.0);
            f.upside_down = true;
            v.push(f);
            let mut g = TItem::new("-7.5\u{b0} kerning AV To Ty", 0, [280.0, 60.0], 14.0);
            g.rotation = -0.1309;
            v.push(g);
        }
        // large: class L and a fill
        2 => {
            v.push(TItem::new("Rag", 0, [10.0, 200.0], 80.0));
            v.push(TItem::new("&@", 1, [250.0, 120.0], 160.0));
        }
        // MTEXT: wrapped, three attachments, spacing
        3 => {
            let mut a = TItem::new("Multi-line text wraps at a width,\nand breaks where it says so.", 1, [20.0, 380.0], 12.0);
            a.multi = Some((180.0, Attach::TopLeft, 1.0));
            v.push(a);
            let mut b = TItem::new("Centred in the middle\nof two lines", 0, [460.0, 300.0], 14.0);
            b.multi = Some((0.0, Attach::MiddleCenter, 1.5));
            v.push(b);
            let mut c = TItem::new("Bottom right, turned\nby twenty degrees", 1, [600.0, 60.0], 11.0);
            c.multi = Some((0.0, Attach::BottomRight, 1.0));
            c.rotation = 0.349;
            v.push(c);
        }
        // translucent, overlapping glyphs (tight kerning), Arabic and Hebrew (bidi, fallback)
        _ => {
            let mut a = TItem::new("Translucent AVAWAY fij", 1, [20.0, 300.0], 30.0);
            a.color = 0x80C0_4020;
            v.push(a);
            let mut b = TItem::new("\u{644}\u{63a}\u{629} \u{639}\u{631}\u{628}\u{64a}\u{629} 123 \u{5e2}\u{5d1}\u{5e8}\u{5d9}\u{5ea}", 2, [20.0, 200.0], 20.0);
            b.color = 0xFF10_3060;
            v.push(b);
            let mut c = TItem::new("Pixels: 14 px", 1, [320.0, 100.0], 14.0);
            c.unit = Unit::Px;
            c.rotation = 0.2;
            v.push(c);
        }
    }
    v
}

/// the atlas of text_drawing(0) and (2): the same bytes on Windows, Linux, the browser and Android
pub const ATLAS_HASH: u64 = 0x3018_1970_a524_8250;
