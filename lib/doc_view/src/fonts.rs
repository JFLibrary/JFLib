// Document-owned fallback fonts. GUI themes and styles never select a document face.
// The SGF1 assets are DejaVu Sans 2.37, byte-identical to jfWord's previous defaults.

use doc_core::{Document, FontFamily, ResolvedFormat};
use alloc::{rc::Rc, string::String, vec::Vec};
use core::cell::RefCell;
use text::{FS_SYN_ITALIC, Text};

const REGULAR: &[u8] = include_bytes!("../assets/font_reg.bin");
const BOLD: &[u8] = include_bytes!("../assets/font_bold.bin");

/// The text registry could not register the fixed document faces or their italic forms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontError {
    /// The registry is full, or a bundled face is invalid.
    Registration,
    /// A synthetic italic face could not be registered.
    Italic,
}

/// Handles to document fonts registered in one `text::Text` instance.
///
/// Construct once per text engine and retain across document changes. Clones share the
/// family cache and registered faces; do not use them with a different text engine.
#[derive(Clone)]
pub struct Fonts {
    ids: [u32; 4],
    cache: Rc<RefCell<FontCache>>,
}
struct FontSet { family: FontFamily, ids: [u32; 4], substituted: bool }
#[derive(Default)]
struct FontCache { sets: Vec<FontSet>, database: text::fontdb::FontDb, scanned: bool }
const BUNDLED: [&str; 3] = ["DejaVu Sans", "DejaVu Serif", "DejaVu Sans Mono"];

impl FontCache {
    fn scan(&mut self) {
        if !self.scanned {
            #[cfg(not(target_arch = "wasm32"))]
            self.database.scan(0);
            self.scanned = true;
        }
    }
}

impl Fonts {
    /// Register the fixed DejaVu Sans fallback, independently of GUI fonts and theme.
    /// Failure rolls back every font registered during this call.
    pub fn new(text: &mut Text) -> Result<Self, FontError> {
        if (0..text::FONT_EMBEDDED).any(|id| !text.valid(id)) {
            return Err(FontError::Registration);
        }
        let regular = text.register(REGULAR);
        if regular < 0 {
            return Err(FontError::Registration);
        }
        let bold = text.register(BOLD);
        if bold < 0 {
            text.unload(regular as u32);
            return Err(FontError::Registration);
        }
        let italic = text.style(regular as u32, FS_SYN_ITALIC);
        let bold_italic = text.style(bold as u32, FS_SYN_ITALIC);
        if italic < 0 || bold_italic < 0 {
            text.unload(regular as u32);
            text.unload(bold as u32);
            return Err(FontError::Italic);
        }
        Ok(Self {
            cache: Rc::new(RefCell::new(FontCache::default())),
            ids: [
                regular as u32,
                bold as u32,
                italic as u32,
                bold_italic as u32,
            ],
        })
    }

    /// Resolve the currently supported character attributes to a registered face.
    pub fn get(&self, format: ResolvedFormat) -> u32 {
        let index = format.bold as usize + 2 * format.italic as usize;
        self.cache.borrow().sets.iter().find(|s| s.family.as_str().eq_ignore_ascii_case(format.font_family.as_str()))
            .map_or(self.ids[index], |s| s.ids[index])
    }

    /// Bundled families on every platform, plus installed native font families.
    /// Browser enumeration does not request access to the user's local fonts.
    pub fn families(&self) -> Vec<String> {
        let mut cache = self.cache.borrow_mut();
        cache.scan();
        let mut names: Vec<_> = BUNDLED.iter().map(|s| String::from(*s)).collect();
        for i in 0..cache.database.family_count() {
            let name = cache.database.family_name(i);
            if FontFamily::new(name).is_ok() && !names.iter().any(|n| n.eq_ignore_ascii_case(name)) { names.push(String::from(name)); }
        }
        names
    }

    /// Register requested faces before measuring. Names survive substitutions unchanged.
    /// The bounded cache is shared by layout, page drawing and thumbnails.
    pub fn prepare(&self, text: &mut Text, document: &Document) {
        for p in document.all_paragraphs() {
            self.ensure(text, document.resolve_paragraph_text(p, Default::default()).font_family);
            for run in &p.content { self.ensure(text, document.resolve_paragraph_text(p, run.format()).font_family); }
        }
    }
    /// True when the requested family is unavailable and a bundled substitute is used.
    pub fn substituted(&self, family: FontFamily) -> bool {
        if family.as_str().eq_ignore_ascii_case(BUNDLED[0]) { return false; }
        self.cache.borrow().sets.iter().find(|s| s.family.as_str().eq_ignore_ascii_case(family.as_str())).is_none_or(|s| s.substituted)
    }
    /// Prepare a family for an isolated style-gallery sample using the same resolver as pages.
    pub fn ensure(&self, text: &mut Text, family: FontFamily) {
        if family.as_str().eq_ignore_ascii_case(BUNDLED[0]) { return; }
        let mut cache = self.cache.borrow_mut();
        if cache.sets.iter().any(|s| s.family.as_str().eq_ignore_ascii_case(family.as_str())) || cache.sets.len() >= 64 { return; }
        let bundled = BUNDLED.iter().position(|s| s.eq_ignore_ascii_case(family.as_str()));
        let mut ids = self.ids;
        let mut substituted = false;
        if let Some(which) = bundled {
            let (regular, bold): (&[u8], &[u8]) = if which == 1 {
                (include_bytes!("../assets/DejaVuSerif.ttf"), include_bytes!("../assets/DejaVuSerif-Bold.ttf"))
            } else { (include_bytes!("../assets/DejaVuSansMono.ttf"), include_bytes!("../assets/DejaVuSansMono-Bold.ttf")) };
            let r = text.load_mem(regular, 0);
            let b = text.load_mem(bold, 0);
            if r >= 0 && b >= 0 {
                let i = text.style(r as u32, FS_SYN_ITALIC);
                let bi = text.style(b as u32, FS_SYN_ITALIC);
                if i >= 0 && bi >= 0 { ids = [r as u32, b as u32, i as u32, bi as u32]; }
                else { substituted = true; text.unload(r as u32); text.unload(b as u32); }
            } else {
                substituted = true;
                if r >= 0 { text.unload(r as u32); }
                if b >= 0 { text.unload(b as u32); }
            }
        } else {
            cache.scan();
            for (index, (weight, italic)) in [(400, false), (700, false), (400, true), (700, true)].iter().enumerate() {
                let id = cache.database.load(text, family.as_str(), *weight, *italic);
                if id >= 0 { ids[index] = id as u32; } else { substituted = true; }
            }
            if substituted {
                let lower = family.as_str().to_ascii_lowercase();
                let substitute = if ["mono", "courier", "consolas"].iter().any(|s| lower.contains(s)) { BUNDLED[2] }
                    else if ["serif", "times", "cambria", "georgia"].iter().any(|s| lower.contains(s)) { BUNDLED[1] }
                    else { BUNDLED[0] };
                drop(cache);
                let fallback = FontFamily::new(substitute).unwrap();
                self.ensure(text, fallback);
                cache = self.cache.borrow_mut();
                ids = cache.sets.iter().find(|s| s.family == fallback).map_or(self.ids, |s| s.ids);
            }
        }
        cache.sets.push(FontSet { family, ids, substituted });
    }

    /// Check that every cached face still belongs to the live text engine.
    pub fn valid(&self, text: &Text) -> bool {
        self.ids.iter().all(|&id| text.valid(id)) && self.cache.borrow().sets.iter().all(|s| s.ids.iter().all(|&id| text.valid(id)))
    }
}
