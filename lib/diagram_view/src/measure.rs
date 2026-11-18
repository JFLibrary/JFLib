// measure -- text measured with the fonts it is drawn with: the display list's Measure of a
// page view, and Metrics, the formula engine's TextMetrics (TEXTWIDTH, TEXTHEIGHT; JD7)
//
//   Metrics::new(&fonts)             measures with a handle on the app's FontSet (shared: faces
//                                    the set finds later count too); Engine::with_metrics(Rc::new(m))
//   metrics.measure(text, font, size, max_width) -> (w, h)   inches, through
//                                    render::measure_text: lines wrapped as a shape's text wraps
//
// Both measure alike: a family resolves through FontSet::pick (its own face, else the face of
// its class), a substitute is stretched by the sample line's width factor (fonts.rs), the
// advance comes from render2d's layout of the line (shaped: kerning, ligatures), in em.  So
// TEXTWIDTH(TheText) is the width the page view draws the same text with.  A family named
// nowhere ("" -- a Font cell the engine cannot name) is Calibri, the layout's default.  The
// faces resolved and the widths measured are kept (at most WIDTHS_MAX widths).

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use diagram_core::formula::TextMetrics;
use diagram_core::render::{self, ApproxMeasure, Face, Measure};
use render2d::{FontId, Fonts, Text, TextStyle, TextStyleId};

use crate::fonts::{sample_width, FontSet, SAMPLE};
use crate::view::SYNTHETIC_ITALIC;

/// how many measured widths a cache keeps
const WIDTHS_MAX: usize = 200_000;

/// a family and style resolved to a font of a render2d Fonts
#[derive(Clone, Copy)]
pub(crate) struct Resolved {
    pub font: FontId,
    /// width factor (the missing font's width over the substitute's)
    pub width: f64,
    pub oblique: f64,
}

/// what measuring keeps between texts (per render2d Fonts: the font ids are its)
#[derive(Default)]
pub(crate) struct FontCache {
    faces: BTreeMap<(String, bool, bool), Option<Resolved>>,
    /// font bytes added to the Fonts -> their id there
    loaded: Vec<(Rc<[u8]>, u32, FontId)>,
    widths: BTreeMap<(String, bool, bool, String), f64>,
    /// families drawn with another font
    pub substituted: Vec<String>,
}

impl FontCache {
    /// a face resolved before
    pub fn known(&self, family: &str, bold: bool, italic: bool) -> Option<Option<Resolved>> {
        self.faces.get(&(family.to_ascii_lowercase(), bold, italic)).copied()
    }
}

/// a FontSet measuring into a render2d Fonts (a scene's, or Metrics' own)
pub(crate) struct Measurer<'a> {
    pub fonts: &'a mut Fonts,
    pub set: &'a FontSet,
    pub cache: &'a mut FontCache,
}

impl Measurer<'_> {
    pub fn resolve(&mut self, face: &Face) -> Option<Resolved> {
        let key = (face.family.to_ascii_lowercase(), face.bold, face.italic);
        if let Some(r) = self.cache.faces.get(&key) {
            return *r;
        }
        let r = self.resolve_new(face);
        self.cache.faces.insert(key, r);
        r
    }

    fn resolve_new(&mut self, face: &Face) -> Option<Resolved> {
        let (f, synth, substituted) = self.set.pick(face.family, face.bold, face.italic)?;
        let font = match self.cache.loaded.iter().find(|l| Rc::ptr_eq(&l.0, &f.data) && l.1 == f.index) {
            Some(l) => l.2,
            None => {
                let font = self.fonts.add(&f.data, f.index).ok()?;
                self.cache.loaded.push((f.data.clone(), f.index, font));
                font
            }
        };
        let mut width = 1.0;
        if substituted {
            if !self.cache.substituted.iter().any(|s| s.eq_ignore_ascii_case(face.family)) {
                self.cache.substituted.push(String::from(face.family));
            }
            if let Some(target) = sample_width(face.family, face.bold) {
                let own = raw_advance(self.fonts, font, SAMPLE);
                if own > 0.0 {
                    width = (target / own).clamp(0.7, 1.3);
                }
            }
        }
        Some(Resolved { font, width, oblique: if synth { SYNTHETIC_ITALIC } else { 0.0 } })
    }
}

/// the advance of a line at an em of 1 (render2d's layout)
fn raw_advance(fonts: &mut Fonts, font: FontId, text: &str) -> f64 {
    let Some(fm) = fonts.metrics(font) else { return 0.0 };
    let styles = [TextStyle::new(font)];
    let mut t = Text::new(text, TextStyleId(0), [0.0, 0.0]);
    t.height = Some(fm.cap_height.max(1e-6));
    match render2d::layout_text(fonts, &styles, &t) {
        Ok(l) => l.lines.first().map_or(0.0, |x| x.width),
        Err(_) => 0.0,
    }
}

impl Measure for Measurer<'_> {
    fn advance(&mut self, face: &Face, text: &str) -> f64 {
        let Some(r) = self.resolve(face) else { return ApproxMeasure.advance(face, text) };
        let key = (String::from(face.family), face.bold, face.italic, String::from(text));
        if let Some(w) = self.cache.widths.get(&key) {
            return *w;
        }
        let w = raw_advance(self.fonts, r.font, text) * r.width;
        if self.cache.widths.len() < WIDTHS_MAX {
            self.cache.widths.insert(key, w);
        }
        w
    }

    fn metrics(&mut self, face: &Face) -> (f64, f64) {
        match self.resolve(face).and_then(|r| self.fonts.metrics(r.font)) {
            Some(m) => (m.ascender, -m.descender),
            None => (0.9, 0.25),
        }
    }
}

/// real text sizes for the formula engine (TEXTWIDTH, TEXTHEIGHT), measured as a page view
/// draws: Engine::with_metrics(Rc::new(Metrics::new(&fonts)))
pub struct Metrics {
    set: FontSet,
    state: RefCell<(Fonts, FontCache)>,
}

impl Metrics {
    /// measuring with the fonts of `fonts` (a handle on the same set)
    pub fn new(fonts: &FontSet) -> Metrics {
        Metrics { set: fonts.clone(), state: RefCell::new((Fonts::new(), FontCache::default())) }
    }

    /// the families measured with a substitute so far
    pub fn substitutions(&self) -> Vec<String> {
        self.state.try_borrow().map(|s| s.1.substituted.clone()).unwrap_or_default()
    }
}

impl TextMetrics for Metrics {
    fn measure(&self, text: &str, font: &str, size: f64, max_width: Option<f64>) -> (f64, f64) {
        let family = if font.trim().is_empty() { "Calibri" } else { font };
        let face = Face { family, bold: false, italic: false };
        match self.state.try_borrow_mut() {
            Ok(mut st) => {
                let (fonts, cache) = &mut *st;
                let mut m = Measurer { fonts, set: &self.set, cache };
                render::measure_text(&mut m, &face, size, text, max_width)
            }
            // (measuring inside a measurement: cannot happen, the estimate then)
            Err(_) => render::measure_text(&mut ApproxMeasure, &face, size, text, max_width),
        }
    }
}
