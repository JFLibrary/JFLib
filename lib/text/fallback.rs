// fallback -- characters a font lacks, drawn with another font (font fallback)
//
//   set_fallback_fonts(ids)        (shaping.rs) the app's own chain, tried first; shaping uses
//                                  it as well
//   set_system_fallback(on)        then system fonts: the families of FAMILIES (this OS's
//                                  list, after the ';' separated families of
//                                  jfgui_FALLBACK_FONTS), found with a FontDb of the font
//                                  folders (scanned on the first missing character; its cache
//                                  is cache://fontdb.fdb, not with jfgui_FONTDIRS) and loaded
//                                  when first needed
//   set_fallback_lang(tag)         the UI language: its fonts (FontConf::lang_families, after
//                                  jfgui_FALLBACK_FONTS) come before FAMILIES, so Han
//                                  characters take the language's forms (ja, zh-Hans, zh-Hant,
//                                  ko) and scripts their usual font
//   system_fallback() -> bool
//   fallback_for(cp) -> Option<font id>   the first font of the chain that has cp (remembered
//                                  per code point, also when none has it)
//   fallback_active() -> bool      a chain is set (run / shaping only look for fallbacks then)
//
// Characters that fonts lack on purpose (controls, zero-width spaces and joiners, bidi marks,
// variation selectors, tags) never fall back.  Fallback glyphs share the baseline and the
// pixel size of the run; ascent, descent and line height stay the first font's.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::fontconf::FontConf;
use crate::fontdb::FontDb;
use crate::Text;

#[cfg(windows)]
const FAMILIES: &[&str] = &[
    "Segoe UI", "Segoe UI Symbol", "Segoe UI Emoji", "Segoe UI Historic", "Microsoft YaHei", "Microsoft JhengHei",
    "Yu Gothic", "Malgun Gothic", "Nirmala UI", "Leelawadee UI", "Ebrima", "Gadugi", "Microsoft Himalaya",
    "Mongolian Baiti", "Myanmar Text", "Javanese Text", "Sylfaen", "Cambria Math", "Arial", "Times New Roman",
    "MS Gothic", "SimSun", "Arial Unicode MS",
];

#[cfg(not(windows))]
const FAMILIES: &[&str] = &[
    "DejaVu Sans", "Noto Sans", "Liberation Sans", "Noto Sans CJK SC", "Noto Sans CJK JP", "Noto Sans CJK KR",
    "Noto Sans CJK TC", "Noto Color Emoji", "Noto Emoji", "Noto Sans Symbols", "Noto Sans Symbols2", "Noto Sans Math",
    "Noto Sans Arabic", "Noto Sans Hebrew", "Noto Sans Devanagari", "Noto Sans Bengali", "Noto Sans Tamil",
    "Noto Sans Thai", "Noto Sans Ethiopic", "Noto Sans Armenian", "Noto Sans Georgian", "WenQuanYi Micro Hei",
    "Droid Sans Fallback", "FreeSans", "FreeSerif", "Unifont", "DejaVu Serif", "DejaVu Sans Mono",
];

const CACHE_MAX: usize = 8192; // remembered code points
const NONE: u32 = u32::MAX;

#[derive(Default)]
pub(crate) struct Fallback {
    /// the app's chain (set_fallback_fonts)
    pub(crate) app: Vec<u32>,
    system: bool,
    db: Option<FontDb>,
    /// the system families in order with their font id (-1: not there / failed; 0 + ...:
    /// loaded); None: not tried yet
    families: Vec<(String, Option<i32>)>,
    cache: BTreeMap<u32, u32>,
    /// set_fallback_lang; the families list is built again when it changes
    lang: String,
    built: bool,
}

/// characters that fonts lack on purpose
fn never(cp: u32) -> bool {
    cp < 0x20
        || (0x7F..=0x9F).contains(&cp)
        || (0x200B..=0x200F).contains(&cp)
        || (0x2028..=0x202E).contains(&cp)
        || (0x2060..=0x206F).contains(&cp)
        || (0xFE00..=0xFE0F).contains(&cp)
        || cp == 0xFEFF
        || (0xE0000..=0xE01EF).contains(&cp)
}

impl Text {
    /// look for missing characters in system fonts too (after set_fallback_fonts' chain)
    pub fn set_system_fallback(&mut self, on: bool) {
        if self.fb.system != on {
            self.fb.system = on;
            self.fb.cache.clear();
        }
    }

    /// the language whose fonts the system fallback tries first ("" none)
    pub fn set_fallback_lang(&mut self, tag: &str) {
        if self.fb.lang != tag {
            self.fb.lang = String::from(tag);
            self.fb.built = false;
            self.fb.cache.clear();
        }
    }

    /// whether system fonts are searched for missing characters
    pub fn system_fallback(&self) -> bool {
        self.fb.system
    }

    /// whether run / shaping look for fallback fonts at all
    pub fn fallback_active(&self) -> bool {
        self.fb.system || !self.fb.app.is_empty()
    }

    // set_fallback_fonts: the app's chain changed
    pub(crate) fn fallback_app(&mut self, ids: &[u32]) {
        self.fb.app = ids.to_vec();
        self.fb.cache.clear();
    }

    /// the first font of the fallback chain that has cp (None: none has it)
    pub fn fallback_for(&mut self, cp: u32) -> Option<u32> {
        if never(cp) || !self.fallback_active() {
            return None;
        }
        if let Some(&id) = self.fb.cache.get(&cp) {
            return (id != NONE).then_some(id);
        }
        let found = self.fallback_search(cp);
        if self.fb.cache.len() >= CACHE_MAX {
            self.fb.cache.clear();
        }
        self.fb.cache.insert(cp, found.unwrap_or(NONE));
        found
    }

    fn fallback_search(&mut self, cp: u32) -> Option<u32> {
        let app = self.fb.app.clone();
        if let Some(id) = app.into_iter().find(|&id| self.valid(id) && self.has(id, cp)) {
            return Some(id);
        }
        if !self.fb.system {
            return None;
        }
        self.fallback_scan();
        for k in 0..self.fb.families.len() {
            let id = match self.fb.families[k].1 {
                Some(id) => id,
                None => {
                    let fam = self.fb.families[k].0.clone();
                    let mut db = self.fb.db.take().unwrap_or_default();
                    let id = db.load(self, &fam, 400, false);
                    self.fb.db = Some(db);
                    self.fb.families[k].1 = Some(id);
                    id
                }
            };
            if id >= 0 && self.has(id as u32, cp) {
                return Some(id as u32);
            }
        }
        None
    }

    // the font folders' faces (once) and the family list (again after a language change)
    fn fallback_scan(&mut self) {
        if self.fb.db.is_some() {
            if !self.fb.built {
                let db = self.fb.db.take().unwrap_or_default();
                self.fallback_families(&db);
                self.fb.db = Some(db);
            }
            return;
        }
        let mut db = FontDb::new();
        // the cache belongs to the system folders; jfgui_FONTDIRS (tests) scans without it
        let cache = plat::env("jfgui_FONTDIRS").is_none().then_some("cache://fontdb.fdb");
        if let Some(c) = cache {
            db.load_cache(c);
        }
        db.scan(0);
        if let Some(c) = cache {
            db.save_cache(c);
        }
        self.fallback_families(&db);
        self.fb.db = Some(db);
    }

    // the installed families of jfgui_FALLBACK_FONTS, the language's and FAMILIES in order
    fn fallback_families(&mut self, db: &FontDb) {
        let mut fams: Vec<String> = plat::env("jfgui_FALLBACK_FONTS")
            .map(|v| v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
            .unwrap_or_default();
        if !self.fb.lang.is_empty() {
            fams.extend(FontConf::system().lang_families(&self.fb.lang));
        }
        fams.extend(FAMILIES.iter().map(|s| String::from(*s)));
        let mut seen: Vec<String> = Vec::new();
        self.fb.families = fams
            .into_iter()
            .filter(|f| {
                let new = !seen.iter().any(|s| s.eq_ignore_ascii_case(f));
                seen.push(f.clone());
                new && db.find(f, 400, false).is_some()
            })
            .map(|f| (f, None))
            .collect();
        self.fb.built = true;
    }
}
