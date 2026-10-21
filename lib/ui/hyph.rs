// hyph -- hyphenation of wrapped texts and text boxes (spell::Hyphenator, text::segment)
//
//   TX_HYPHENATE (text style, with TX_WRAP), TB_HYPHENATE (textbox, with TB_MULTI | TB_WRAP)
//        a word that does not fit at the end of a line is broken at its last hyphenation
//        point that fits, and a hyphen is drawn there.  Soft hyphens (U+00AD) in the text
//        work in every wrapped text, with or without the flag: invisible inside a line, a
//        hyphen where the line breaks after one
//   hyph_language(tag)          the language: a BCP 47 tag; "" (default) = font_language(),
//                               the user's tag of the UI language
//   next_hyph_language(tag)     the language of the next wrapped text (TX_WRAP) or multi-line
//                               box only (it counts when that one hyphenates)
//   hyph_sources(SRC_*)         where dictionaries come from (spell crate; 0 = the default:
//                               everything in window mode, only app:// in headless runs, so
//                               scripts never depend on the machine)
//   hyph_set(tag, hyphenator)   the app's own patterns for a language (before any source)
//   hyphenator(tag) -> Option<Arc<Hyphenator>>   the patterns for a tag ("": the current
//                               language), found (spell::find_hyphen: hyph_xx_YY.dic of
//                               LibreOffice, /usr/share/hyphen, config://hyphen,
//                               app://hyphen) and read on first use, then kept
//
// A dictionary is read on the UI thread the first time a language is needed (de_DE, 1 MB:
// some 40 ms); a language without one keeps texts unhyphenated (soft hyphens still work).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use spell::{find_hyphen, Hyphenator, SRC_ALL, SRC_APP};

use crate::Ui;

/// the UI's hyphenation state
#[derive(Default)]
pub(crate) struct S {
    lang: String,
    next: Option<String>,
    sources: u32,
    /// the app's own patterns (tag in lower case with '_')
    own: Vec<(String, Arc<Hyphenator>)>,
    /// tag (lower case with '_') -> what was found for it
    cache: BTreeMap<String, Option<Arc<Hyphenator>>>,
}

// "de-DE" -> "de_de"
fn norm(tag: &str) -> String {
    tag.trim().to_ascii_lowercase().replace('-', "_")
}

impl Ui {
    /// the hyphenation language: a BCP 47 tag ("de-DE"); "" = the UI language's
    pub fn hyph_language(&mut self, tag: &str) {
        if self.m.hyph.lang != tag {
            self.m.hyph.lang = String::from(tag);
            self.repass();
        }
    }

    /// the language of the next wrapped text or multi-line box only (it counts when that one
    /// hyphenates: TX_HYPHENATE, TB_HYPHENATE)
    pub fn next_hyph_language(&mut self, tag: &str) {
        self.m.hyph.next = Some(String::from(tag));
    }

    /// where hyphenation dictionaries come from (spell::SRC_*; 0 = the default)
    pub fn hyph_sources(&mut self, sources: u32) {
        if self.m.hyph.sources != sources {
            self.m.hyph.sources = sources;
            self.m.hyph.cache.clear();
            self.repass();
        }
    }

    /// the app's own hyphenation patterns for a language tag ("de", "en-US")
    pub fn hyph_set(&mut self, tag: &str, h: Hyphenator) {
        let t = norm(tag);
        self.m.hyph.own.retain(|o| o.0 != t);
        self.m.hyph.own.push((t, Arc::new(h)));
        self.m.hyph.cache.clear();
        self.repass();
    }

    /// the hyphenation patterns for a tag ("": the current language), None when there are none
    pub fn hyphenator(&mut self, tag: &str) -> Option<Arc<Hyphenator>> {
        let tag = if tag.is_empty() {
            if self.m.hyph.lang.is_empty() { crate::font_language() } else { self.m.hyph.lang.clone() }
        } else {
            String::from(tag)
        };
        let t = norm(&tag);
        if let Some(h) = self.m.hyph.cache.get(&t) {
            return h.clone();
        }
        // the app's own: the tag, else the same language
        let primary = String::from(t.split('_').next().unwrap_or(""));
        let own = self.m.hyph.own.iter().find(|o| o.0 == t).or_else(|| self.m.hyph.own.iter().find(|o| o.0.split('_').next() == Some(primary.as_str())));
        let found = match own {
            Some(o) => Some(o.1.clone()),
            None => {
                let sources = match self.m.hyph.sources {
                    0 if self.headless => SRC_APP,
                    0 => SRC_ALL,
                    s => s,
                };
                find_hyphen(&tag, sources).and_then(|(path, _)| Hyphenator::load(&path)).map(Arc::new)
            }
        };
        self.m.hyph.cache.insert(t, found.clone());
        found
    }

    // (crate) the patterns for the widget being made: next_hyph_language, else the language
    // (taken when `on`: the widget hyphenates; the pending language is used up either way)
    pub(crate) fn hyph_take(&mut self, on: bool) -> Option<Arc<Hyphenator>> {
        let next = self.m.hyph.next.take();
        if !on {
            return None;
        }
        self.hyphenator(next.as_deref().unwrap_or(""))
    }
}
