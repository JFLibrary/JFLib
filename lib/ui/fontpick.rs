// fontpick -- the system's fonts in the UI: the font database and its rules, loading a
// choice, text in such a font, a font picker and a font dialog
//
//   FontSel { family, weight, italic, size, axes }   a choice: a family (also a generic one:
//                                    "sans-serif", "serif", "monospace" ...), 100..900, italic,
//                                    the size in points, axis values of a variable family
//                                    (FontSel::new: none; axis / set_axis)
//   ui.fonts() -> &FontDb            the installed fonts, scanned on first use (cache
//                                    cache://fontdb.fdb; jfgui_FONTDIRS: these folders, no cache)
//   ui.font_conf() -> &FontConf      generic families, substitutes, the languages' fonts
//                                    (text::fontconf: a light fontconfig)
//   ui.font_load(&sel) -> Option<u32>   resolve (FontDb::resolve in the UI language) and load
//                                    into ui.text: a font id for set_font / cmd_text; each
//                                    family / weight / italic once.  A variable face: its
//                                    instance (FontDb::load_var): weight -> wght, italic -> ital
//                                    / slnt, the size -> opsz, the axes named win; the instances
//                                    of the VAR_KEEP most recent such choices and the app
//                                    theme's faces stay loaded (older ones are unloaded, their
//                                    ids may be reused)
//   ui.font_resolved(&sel) -> String the installed family font_load takes ("" none)
//   ui.font_register(family, weight, italic, font)   a font of the app's own (load_mem) under
//                                    a family name for font_load; font_registered() lists them
//   ui.text_in_font(font, size_pt, s)   one line of text in a font id at a size in points
//   ui.font_picker(id, &mut sel) -> EV  a drop-down button with the family (a search box and
//                                    the families open below it: typing filters and takes
//                                    the first match, Up / Down / a click choose) and a size
//                                    box in points; EV_ACT when sel changed
//   ui.font_dialog(id, &open, &mut sel) -> EV   a modal dialog window "Schriftart": search,
//                                    families, the family's styles, sizes, a preview in the
//                                    font, OK / Abbrechen; works on a copy of sel and writes it
//                                    back on OK (EV_ACT); closing clears open.  A variable
//                                    family lists its named instances as styles and gets a
//                                    slider and a number box per axis (fvar's range and name;
//                                    hidden axes not), opsz with "Automatisch" (follows the size)
//   font_language() -> String        the language the fonts follow: the user's tag of the UI
//                                    language (ja-JP, zh-Hant-TW), else its code
//
// The runtime hands font_language() to the system fallback (text::fallback, again when the
// UI language changes), so missing Han characters take the forms of the language.  Ids:
// the scope of the widget's id, then "family", "search", "list", "styles", "size",
// "sizes", the axes' "axis" scope with per axis id_n(k) (slider), id_n(k + 100) (number
// box), "opsz_auto" (ids.rs).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use text::fontconf::FontConf;
use text::fontdb::{FontDb, FDFF_ITALIC, FDFF_OBLIQUE, FDFF_VAR};
use text::VarAxis;

use super::consts::*;
use super::value::RangeF;
use super::win::Shared;
use super::Ui;

const ROW_H: u32 = 28;
const SEARCH_CAP: u32 = 128;
const SIZES: [f64; 16] = [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0, 36.0, 48.0, 72.0, 96.0];
const RANGE_PT: RangeF = RangeF::new(1.0, 1638.0, 1.0, 1, 0);
const SAMPLE: &str = "AaBbYyZz 0123456789";
/// the instances of variable choices font_load keeps loaded
const VAR_KEEP: usize = 64;

/// a font choice (see the module header)
#[derive(Clone, Debug, PartialEq)]
pub struct FontSel {
    pub family: String,
    pub weight: u32,
    pub italic: bool,
    /// points
    pub size: f64,
    /// axis values of a variable family in user units by tag (`(*b"wdth", 87.5)`); the axes
    /// not named follow weight (wght), italic (ital, slnt) and size (opsz), else their
    /// defaults; families that are not variable ignore them
    pub axes: Vec<([u8; 4], f32)>,
}

impl FontSel {
    /// regular, upright, no axis values
    pub fn new(family: &str, size: f64) -> FontSel {
        FontSel { family: String::from(family), weight: 400, italic: false, size, axes: Vec::new() }
    }

    /// the value `axes` names for a tag (None: the tag follows weight / italic / size or its
    /// default)
    pub fn axis(&self, tag: [u8; 4]) -> Option<f32> {
        self.axes.iter().find(|a| a.0 == tag).map(|a| a.1)
    }

    /// name an axis value (replacing the tag's); NaN takes the tag out again
    pub fn set_axis(&mut self, tag: [u8; 4], v: f32) {
        self.axes.retain(|a| a.0 != tag);
        if !v.is_nan() {
            self.axes.push((tag, v));
        }
    }
}

impl Default for FontSel {
    fn default() -> FontSel {
        FontSel::new("sans-serif", 11.0)
    }
}

// a style of the dialog's list: a face, or a named instance of a variable face (face index,
// label, weight, italic, the axis values it names)
struct StyleRow {
    face: i32,
    name: String,
    weight: u32,
    italic: bool,
    axes: Vec<([u8; 4], f32)>,
}

// a variable choice of font_entry: family, weight, italic, size bits, axes (tag, value bits)
type VarKey = (String, u32, bool, u64, Vec<([u8; 4], u32)>);

// the state of one picker / dialog between frames
#[derive(Default)]
struct Pick {
    open: bool,
    search: String,
    filt: Vec<u32>,
    sel: i32,
    style: i32,
    size: i32,
    work: FontSel,
    rect: Option<Shared<WinRect>>,
    fresh: bool, // the dialog was opened: reset its lists inside its window
}

/// shared state (Ui::m.fonts)
#[derive(Default)]
pub(crate) struct S {
    db: Option<FontDb>,
    conf: Option<FontConf>,
    loaded: BTreeMap<(String, u32, bool), (i32, String)>,
    /// choices on variable faces, oldest first: (key, font id, family)
    vloaded: Vec<(VarKey, i32, String)>,
    picks: BTreeMap<u32, Pick>,
    /// the UI language the fallback follows (i18n::language() + 1; 0: not set yet)
    lang_n: u32,
    /// (ROADMAP §6) font_register: the families of the app's own fonts, in order
    registered: Vec<String>,
}

/// the language the fonts follow: the user's tag of the UI language, else its code
pub fn font_language() -> String {
    let code = super::i18n::language_code();
    super::i18n::user_languages()
        .into_iter()
        .find(|u| u.split(['-', '_']).next().is_some_and(|p| p.eq_ignore_ascii_case(&code)))
        .unwrap_or(code)
}

// the values of named axes as a key (f32 bits)
fn var_key(sel: &FontSel) -> VarKey {
    let mut axes: Vec<([u8; 4], u32)> = sel.axes.iter().map(|a| (a.0, a.1.to_bits())).collect();
    axes.sort_unstable();
    (sel.family.to_lowercase(), sel.weight, sel.italic, sel.size.to_bits(), axes)
}

// the axis values a style row names: those that the mapping of weight / italic does not give
// (wght never: the weight; opsz only when the instances differ in it, else it follows the size)
fn row_axes(axes: &[VarAxis], coords: &[f32], italic: bool, opsz_named: bool) -> Vec<([u8; 4], f32)> {
    let has_ital = axes.iter().any(|a| &a.tag == b"ital" && a.max >= 1.0);
    let mut out = Vec::new();
    for (a, &v) in axes.iter().zip(coords) {
        let mapped = match &a.tag {
            b"wght" => continue,
            b"opsz" if !opsz_named => continue,
            b"opsz" => f32::NAN,
            b"ital" if italic && a.max >= 1.0 => 1.0,
            b"slnt" if italic && !has_ital && a.min < 0.0 => a.min,
            _ => a.def,
        };
        if !(mapped - v).abs().le(&0.001) {
            out.push((a.tag, v));
        }
    }
    out
}

// the slider range of an axis: whole steps for wide ranges, tenths / hundredths for narrow
fn axis_range(a: &VarAxis) -> RangeF {
    let span = (a.max - a.min) as f64;
    let (step, dec) = if span >= 50.0 { (1.0, 0) } else if span >= 2.0 { (0.1, 1) } else { (0.01, 2) };
    RangeF::new(a.min as f64, a.max as f64, step, dec, 0)
}

fn has_ci(hay: &str, needle: &str) -> bool {
    needle.is_empty() || hay.to_lowercase().contains(&needle.to_lowercase())
}

// the label of a style: the face's own name (FN_STYLE), else from weight and slant
fn style_label(weight: u32, italic: bool) -> &'static str {
    let tr = super::i18n::tr;
    match (weight >= 600, italic) {
        (false, false) => tr("Standard"),
        (false, true) => tr("Kursiv"),
        (true, false) => tr("Fett"),
        (true, true) => tr("Fett Kursiv"),
    }
}

impl Ui {
    fn fonts_db(&mut self) -> &mut FontDb {
        self.m.fonts.db.get_or_insert_with(|| {
            let mut db = FontDb::new();
            let cache = plat::env("jfgui_FONTDIRS").is_none().then_some("cache://fontdb.fdb");
            if let Some(c) = cache {
                db.load_cache(c);
            }
            #[cfg(not(target_arch = "wasm32"))]
            db.scan(0);
            if let Some(c) = cache {
                db.save_cache(c);
            }
            db
        })
    }

    /// the installed fonts (scanned on first use)
    pub fn fonts(&mut self) -> &FontDb {
        self.fonts_db()
    }

    /// the rules for generic families, substitutes and languages
    pub fn font_conf(&mut self) -> &FontConf {
        self.m.fonts.conf.get_or_insert_with(FontConf::system)
    }

    // (font_load, font_resolved) the font of a choice: (id, family), loaded once; a variable
    // face's instance for weight, italic, size and axes (the VAR_KEEP most recent kept)
    fn font_entry(&mut self, sel: &FontSel) -> (i32, String) {
        let key = (sel.family.to_lowercase(), sel.weight, sel.italic);
        if let Some((id, fam)) = self.m.fonts.loaded.get(&key).cloned()
            && id >= 0
            && self.text.valid(id as u32)
        {
            // a static face, or the app's own font (a variable one varies with the axes)
            if sel.axes.is_empty() || self.text.var_axes(id as u32).is_empty() {
                return (id, fam);
            }
            let v = self.text.vary(id as u32, &sel.axes);
            return (if v >= 0 { v } else { id }, fam);
        }
        let vk = var_key(sel);
        if let Some(e) = self.m.fonts.vloaded.iter().find(|e| e.0 == vk)
            && e.1 >= 0
            && self.text.valid(e.1 as u32)
        {
            return (e.1, e.2.clone());
        }
        self.fonts_db();
        self.font_conf();
        let lang = font_language();
        let s = &mut self.m.fonts;
        let (Some(db), Some(conf)) = (s.db.as_mut(), s.conf.as_ref()) else { return (-1, String::new()) };
        let Some((fi, syn, fam)) = db.resolve(conf, &sel.family, &lang, sel.weight, sel.italic) else {
            s.loaded.insert(key, (-1, String::new()));
            return (-1, String::new());
        };
        if db.face(fi).is_none_or(|f| f.flags & FDFF_VAR == 0) {
            let id = db.load_face(&mut self.text, fi, syn);
            s.loaded.insert(key, (id, fam.clone()));
            return (id, fam);
        }
        let id = db.load_var(&mut self.text, fi, syn, sel.weight, sel.italic, Some(sel.size as f32), &sel.axes);
        s.vloaded.retain(|e| e.0 != vk);
        s.vloaded.push((vk, id, fam.clone()));
        if s.vloaded.len() > VAR_KEEP {
            // the oldest choice: its instance goes unless another choice or the app theme's
            // faces still have it
            let (_, old, _) = s.vloaded.remove(0);
            let used = s.vloaded.iter().any(|e| e.1 == old)
                || s.loaded.values().any(|e| e.0 == old)
                || self.look.fonts.contains(&(old as u32));
            if old >= 0 && !used && !self.text.var_coords(old as u32).is_empty() {
                self.text.unload(old as u32);
            }
        }
        (id, fam)
    }

    /// the font of a choice loaded into ui.text -> its id (None: no installed font fits)
    pub fn font_load(&mut self, sel: &FontSel) -> Option<u32> {
        let (id, _) = self.font_entry(sel);
        (id >= 0).then_some(id as u32)
    }

    /// the installed family font_load takes for a choice ("" none)
    pub fn font_resolved(&mut self, sel: &FontSel) -> String {
        self.font_entry(sel).1
    }

    /// a font of the app's own (an id of ui.text.load_mem) under a family name: font_load
    /// takes it for that family, weight and slant without asking the installed fonts; the
    /// rich-text toolbar lists the registered families (rich_edit.rs)
    pub fn font_register(&mut self, family: &str, weight: u32, italic: bool, font: u32) {
        let s = &mut self.m.fonts;
        s.loaded.insert((family.to_lowercase(), weight, italic), (font as i32, String::from(family)));
        if !s.registered.iter().any(|f| f.eq_ignore_ascii_case(family)) {
            s.registered.push(String::from(family));
        }
    }

    /// the families of font_register, in the order of their first registration
    pub fn font_registered(&self) -> Vec<String> {
        self.m.fonts.registered.clone()
    }

    // (rich_edit.rs) a registered face of a family: exactly (weight, italic) -> its id
    pub(crate) fn font_registered_face(&self, family: &str, weight: u32, italic: bool) -> Option<u32> {
        let s = &self.m.fonts;
        if !s.registered.iter().any(|f| f.eq_ignore_ascii_case(family)) {
            return None;
        }
        s.loaded.get(&(family.to_lowercase(), weight, italic)).filter(|e| e.0 >= 0 && self.text.valid(e.0 as u32)).map(|e| e.0 as u32)
    }

    // (rich_edit.rs) the family is one of font_register
    pub(crate) fn font_is_registered(&self, family: &str) -> bool {
        self.m.fonts.registered.iter().any(|f| f.eq_ignore_ascii_case(family))
    }

    /// one line of text in font id `font` at size_pt points (the body text colour)
    pub fn text_in_font(&mut self, font: u32, size_pt: f64, s: &str) {
        let s = super::basic::cstr(s);
        let dpi = self.wp.rt.dpi as f64;
        let px = (size_pt * 4.0 / 3.0 * dpi / 96.0 * 256.0).clamp(256.0, 2048.0 * 256.0) as i32;
        let prev = self.text.get_font();
        self.text.set_font(font, px);
        let w = self.text.width(s);
        let asc = self.text.ascent();
        let lh = self.text.line_h().max(asc + self.text.descent());
        let r = self.place(w, lh);
        let c = self.text_color(TS_BODY);
        let y = r[1] + ((r[3] - lh) >> 1) + asc;
        self.cmd_text(r[0], y, s.as_bytes(), font, px, c, r[2]);
        self.text.set_font(prev.0, prev.1);
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(s), 0);
    }

    /// (frame_begin) the fallback follows the UI language
    pub(crate) fn fonts_frame(&mut self) {
        let n = super::i18n::language() + 1;
        if self.m.fonts.lang_n != n {
            self.m.fonts.lang_n = n;
            self.text.set_fallback_lang(&font_language());
        }
    }

    // the indices of the families whose name holds the search text
    fn font_filter(&mut self, search: &str) -> Vec<u32> {
        let db = self.fonts_db();
        (0..db.family_count()).filter(|&i| has_ci(db.family_name(i), search)).map(|i| i as u32).collect()
    }

    // the index of a family by name (-1: not installed)
    fn font_family_index(&mut self, family: &str) -> i32 {
        let db = self.fonts_db();
        (0..db.family_count()).find(|&i| db.family_name(i).eq_ignore_ascii_case(family)).map_or(-1, |i| i as i32)
    }

    // the search box and the family list of a picker or dialog (p: its state) -> a family
    // was chosen (p.work.family); close: a click closes the popup
    fn font_family_list(&mut self, p: &mut Pick, w: i32, h: i32, close: bool) -> bool {
        let mut changed = false;
        self.next_size(w, 0);
        let mut s = core::mem::take(&mut p.search);
        let ev = self.textbox(self.id("search"), Some(super::i18n::tr("Suchen")), &mut s, SEARCH_CAP);
        p.search = s;
        if ev & EV_ACT != 0 {
            p.filt = self.font_filter(&p.search.clone());
            let lid = self.id("list");
            self.scroll_to(lid, -1, 0);
            p.sel = if p.filt.is_empty() { -1 } else { 0 };
            if let Some(&f) = p.filt.first() {
                p.work.family = String::from(self.fonts_db().family_name(f as usize));
                changed = true;
            }
        }
        let n = p.filt.len() as i32;
        let names: Vec<String> = {
            let db = self.fonts_db();
            p.filt.iter().map(|&f| String::from(db.family_name(f as usize))).collect()
        };
        self.next_size(w, h);
        let lid = self.id("list");
        let mut sel = p.sel;
        let (first, end) = self.list_begin(lid, &mut sel, n, ROW_H);
        let mut clicked = -1;
        for row in first..end.min(n) {
            if self.list_item(row, &names[row as usize]) & EV_ACT != 0 {
                clicked = row;
            }
        }
        let (lev, _) = self.list_end(&mut sel);
        p.sel = sel;
        if clicked >= 0 {
            p.sel = clicked;
            p.work.family = names[clicked as usize].clone();
            changed = true;
            if close {
                self.popup_close();
            }
        } else if lev & EV_ACT != 0 && (0..n).contains(&p.sel) {
            p.work.family = names[p.sel as usize].clone();
            changed = true;
        }
        changed
    }

    // a picker / dialog opened: the whole list, the current family selected and in view
    fn font_list_reset(&mut self, p: &mut Pick) {
        p.search.clear();
        p.filt = self.font_filter("");
        let fi = self.font_family_index(&p.work.family);
        p.sel = p.filt.iter().position(|&f| f as i32 == fi).map_or(-1, |k| k as i32);
        let lid = self.id("list");
        self.scroll_to(lid, -1, ((p.sel - 4).max(0) as u32 * ROW_H) as i32);
    }

    /// the font picker (see the module header) -> EV_ACT when sel changed
    pub fn font_picker(&mut self, id: u32, sel: &mut FontSel) -> u32 {
        let mut p = self.m.fonts.picks.remove(&id).unwrap_or_default();
        self.id_push(id);
        let mut out = 0;
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        let bid = self.id("family");
        self.next_size(220, 0);
        let label = sel.family.clone();
        let ev = self.dropdown_button(bid, &label);
        let open = ev & EV_OPEN != 0;
        if open != p.open {
            p.open = open;
            p.work = sel.clone();
            if open {
                self.font_list_reset(&mut p);
                let sid = self.id("search");
                self.focus(sid);
            }
        }
        if self.flyout_begin(bid, FL_BELOW | FL_MINW) && self.font_family_list(&mut p, 260, 320, true) {
            if !sel.family.eq_ignore_ascii_case(&p.work.family) {
                sel.axes.clear(); // the axis values of the old family
            }
            sel.family = p.work.family.clone();
            out |= EV_ACT;
        }
        self.flyout_end();
        let mut size = sel.size;
        self.next_size(64, 0);
        if self.number_box_f(self.id("size"), &mut size, &RANGE_PT) & EV_ACT != 0 && size != sel.size {
            sel.size = size;
            out |= EV_ACT;
        }
        self.box_end();
        self.id_pop();
        self.m.fonts.picks.insert(id, p);
        out
    }

    /// the font dialog (see the module header) -> EV_ACT when OK took the choice
    pub fn font_dialog(&mut self, id: u32, open: &Shared<i32>, sel: &mut FontSel) -> u32 {
        let mut p = self.m.fonts.picks.remove(&id).unwrap_or_default();
        let rect = p.rect.get_or_insert_with(|| Shared::new(WinRect::new(0, 0, 600, 0, 0))).clone();
        let mut out = 0;
        self.id_push(id);
        let shown = open.get() != 0;
        if shown && !p.open {
            p.work = sel.clone();
            p.style = -1;
            p.size = SIZES.iter().position(|&s| s == p.work.size).map_or(-1, |k| k as i32);
            p.fresh = true;
        }
        p.open = shown;
        if self.window_begin(id, super::i18n::tr("Schriftart"), Some(open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
            if p.fresh {
                p.fresh = false;
                self.font_list_reset(&mut p);
                let sid = self.id("search");
                self.focus(sid);
            }
            self.box_begin(0, BOX_V | PAD_12, 10);
            self.box_begin(0, BOX_H, 12);
            // families
            self.next_size(-1, 0);
            self.box_begin(0, BOX_V, 6);
            self.text(TS_BODY_STRONG, super::i18n::tr("Schriftart"));
            let fam0 = p.work.family.clone();
            if self.font_family_list(&mut p, -1, 220, false) {
                p.style = -1;
                if !fam0.eq_ignore_ascii_case(&p.work.family) {
                    p.work.axes.clear(); // the axis values of the old family
                }
            }
            self.box_end();
            // the family's styles (the four synthetic ones for a family that is not installed;
            // a variable face's named instances)
            self.next_size(150, 0);
            self.box_begin(0, BOX_V, 6);
            self.text(TS_BODY_STRONG, super::i18n::tr("Schriftschnitt"));
            let fi = self.font_family_index(&p.work.family);
            let styles = self.font_styles(fi);
            let var = styles.iter().any(|r| r.face >= 0 && !r.axes.is_empty())
                || self.font_var_face(&p.work).is_some();
            if var {
                // any axis value is a choice: a style is selected when it is exactly that
                p.style = self.font_style_match(&p.work, &styles);
            } else if p.style < 0 {
                // the face closest to the choice
                p.style = styles
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, s)| (s.italic != p.work.italic) as i32 * 10000 + (s.weight as i32 - p.work.weight as i32).abs())
                    .map_or(-1, |(k, _)| k as i32);
                // the choice snaps to that face (what the list shows is what OK takes)
                if let Some(st) = styles.get(p.style.max(0) as usize).filter(|_| p.style >= 0) {
                    p.work.weight = st.weight;
                    p.work.italic = st.italic;
                }
            }
            let sid = self.id("styles");
            let mut ssel = p.style;
            let n = styles.len() as i32;
            self.next_size(-1, 258);
            let (first, end) = self.list_begin(sid, &mut ssel, n, ROW_H);
            for row in first..end.min(n) {
                self.list_item(row, &styles[row as usize].name);
            }
            self.list_end(&mut ssel);
            if ssel != p.style && (0..n).contains(&ssel) {
                p.style = ssel;
                let st = &styles[ssel as usize];
                p.work.weight = st.weight;
                p.work.italic = st.italic;
                p.work.axes = st.axes.clone();
            }
            self.box_end();
            // sizes
            self.next_size(90, 0);
            self.box_begin(0, BOX_V, 6);
            self.text(TS_BODY_STRONG, super::i18n::tr("Größe"));
            let mut size = p.work.size;
            self.next_size(-1, 0);
            if self.number_box_f(self.id("size"), &mut size, &RANGE_PT) & EV_ACT != 0 {
                p.work.size = size;
                p.size = SIZES.iter().position(|&s| s == size).map_or(-1, |k| k as i32);
            }
            let zid = self.id("sizes");
            let mut zsel = p.size;
            self.next_size(-1, 220);
            let (first, end) = self.list_begin(zid, &mut zsel, SIZES.len() as i32, ROW_H);
            for row in first..end.min(SIZES.len() as i32) {
                self.list_item(row, &format!("{}", SIZES[row as usize]));
            }
            self.list_end(&mut zsel);
            if zsel != p.size && (0..SIZES.len() as i32).contains(&zsel) {
                p.size = zsel;
                p.work.size = SIZES[zsel as usize];
            }
            self.box_end();
            self.box_end();
            // a variable family's axes
            if var {
                self.font_axes(&mut p.work);
            }
            // preview
            self.text(TS_BODY_STRONG, super::i18n::tr("Vorschau"));
            self.next_size(-1, 96);
            self.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_CENTER | pad(12), 4);
            let work = p.work.clone();
            match self.font_load(&work) {
                Some(f) => self.text_in_font(f, work.size, SAMPLE),
                None => self.text(TS_BODY | TX_SECONDARY, super::i18n::tr("Keine passende Schrift installiert")),
            }
            self.box_end();
            let fam = self.font_resolved(&work);
            if !fam.is_empty() && !fam.eq_ignore_ascii_case(&work.family) {
                let note = format!("{} {}", super::i18n::tr("Angezeigt in:"), fam);
                self.text(TS_CAPTION | TX_SECONDARY, &note);
            }
            self.box_end();
            let r = self.dialog_buttons(Some(super::i18n::tr("OK")), None, Some(super::i18n::tr("Abbrechen")));
            if r == DLG_PRIMARY {
                *sel = p.work.clone();
                out |= EV_ACT;
            }
        }
        self.window_end();
        self.id_pop();
        self.m.fonts.picks.insert(id, p);
        out
    }

    // the dialog's styles of family fi: each face, a variable face's named instances (-1: the
    // four synthetic styles of a family that is not installed)
    fn font_styles(&mut self, fi: i32) -> Vec<StyleRow> {
        if fi < 0 {
            return [(400, false), (400, true), (700, false), (700, true)]
                .iter()
                .map(|&(w, it)| StyleRow { face: -1, name: String::from(style_label(w, it)), weight: w, italic: it, axes: Vec::new() })
                .collect();
        }
        let db = self.fonts_db();
        let mut rows = Vec::new();
        for k in db.family_range(fi as usize) {
            let Some(f) = db.face(k) else { continue };
            let (fw, it, style) = (f.weight as u32, f.flags & (FDFF_ITALIC | FDFF_OBLIQUE) != 0, f.style.clone());
            let name = if style.is_empty() { String::from(style_label(fw, it)) } else { style };
            match db.variations(k) {
                Some(v) if !v.instances.is_empty() => {
                    // opsz is part of a named instance when the instances differ in it
                    let ko = v.axes.iter().position(|a| &a.tag == b"opsz");
                    let opsz_named = ko.is_some_and(|ko| v.instances.iter().any(|i| i.coords.get(ko) != v.instances[0].coords.get(ko)));
                    for ins in &v.instances {
                        let val = |tag: &[u8; 4]| v.axes.iter().position(|a| &a.tag == tag).and_then(|i| ins.coords.get(i).copied());
                        let weight = val(b"wght").map_or(fw, |w| (w + 0.5).clamp(1.0, 1000.0) as u32);
                        let italic = it || val(b"ital").is_some_and(|x| x >= 0.5) || val(b"slnt").is_some_and(|x| x < 0.0);
                        let label = if ins.name.is_empty() { String::from(style_label(weight, italic)) } else { ins.name.clone() };
                        let axes = row_axes(&v.axes, &ins.coords, italic, opsz_named);
                        rows.push(StyleRow { face: k as i32, name: label, weight, italic, axes });
                    }
                }
                _ => rows.push(StyleRow { face: k as i32, name, weight: fw, italic: it, axes: Vec::new() }),
            }
        }
        rows
    }

    // the variable face a choice takes (its index), None: not a variable face
    fn font_var_face(&mut self, sel: &FontSel) -> Option<usize> {
        let db = self.fonts_db();
        let (fi, _) = db.find(&sel.family, sel.weight, sel.italic)?;
        (db.face(fi)?.flags & FDFF_VAR != 0).then_some(fi)
    }

    // the style row that is exactly the choice (-1: none)
    fn font_style_match(&mut self, sel: &FontSel, rows: &[StyleRow]) -> i32 {
        let face = self.font_var_face(sel).map_or(-1, |f| f as i32);
        let size = Some(sel.size as f32);
        let db = self.fonts_db();
        let cur = if face >= 0 { db.var_values(face as usize, sel.weight, sel.italic, size, &sel.axes).map(|v| v.0) } else { None };
        for (k, r) in rows.iter().enumerate() {
            let same = match &cur {
                Some(c) if r.face == face => {
                    db.var_values(face as usize, r.weight, r.italic, size, &r.axes).is_some_and(|v| v.0.iter().zip(c).all(|(a, b)| (a - b).abs() < 0.01))
                }
                Some(_) => false,
                None => r.weight == sel.weight && r.italic == sel.italic,
            };
            if same {
                return k as i32;
            }
        }
        -1
    }

    // the dialog's axes of a variable choice: per axis its name, a slider and a number box
    // (fvar's range); opsz with "Automatisch" (it follows the size)
    fn font_axes(&mut self, w: &mut FontSel) {
        let Some(face) = self.font_var_face(w) else { return };
        let size = Some(w.size as f32);
        let db = self.fonts_db();
        let Some(v) = db.variations(face).cloned() else { return };
        let Some((vals, _)) = db.var_values(face, w.weight, w.italic, size, &w.axes) else { return };
        self.text(TS_BODY_STRONG, super::i18n::tr("Achsen"));
        self.id_push_str("axis");
        self.gridbox_begin(0, &[0, -1, 80, 0], 0, 8);
        let mut changed: Option<(usize, f32)> = None;
        for (k, a) in v.axes.iter().enumerate() {
            if a.hidden {
                continue;
            }
            let rf = axis_range(a);
            self.text(TS_BODY, &a.name);
            let mut x = vals[k] as f64;
            self.next_size(-1, 0);
            if self.slider_f(self.id_n(k as u32), &mut x, &rf) & EV_ACT != 0 {
                changed = Some((k, x as f32));
            }
            let mut y = vals[k] as f64;
            self.next_size(-1, 0);
            if self.number_box_f(self.id_n(k as u32 + 100), &mut y, &rf) & EV_ACT != 0 && y != vals[k] as f64 {
                changed = Some((k, y as f32));
            }
            if &a.tag == b"opsz" {
                let mut auto = w.axis(*b"opsz").is_none() as i32;
                if self.checkbox(self.id("opsz_auto"), super::i18n::tr("Automatisch"), &mut auto) & EV_ACT != 0 {
                    w.set_axis(*b"opsz", if auto != 0 { f32::NAN } else { vals[k] });
                }
            } else {
                self.text(TS_BODY, "");
            }
        }
        self.gridbox_end();
        self.id_pop();
        let Some((k, x)) = changed else { return };
        let a = &v.axes[k];
        let x = x.clamp(a.min, a.max);
        match &a.tag {
            b"wght" => {
                w.weight = (x + 0.5).clamp(1.0, 1000.0) as u32;
                w.set_axis(a.tag, if (x - w.weight as f32).abs() < 0.001 { f32::NAN } else { x });
            }
            b"opsz" => w.set_axis(a.tag, x),
            t => {
                if t == b"ital" {
                    w.italic = x >= 0.5;
                } else if t == b"slnt" && !v.axes.iter().any(|b| &b.tag == b"ital") {
                    w.italic = x < 0.0;
                }
                w.set_axis(a.tag, x);
                // a value the mapping of weight / italic gives needs no entry
                let face2 = self.font_var_face(w);
                let db = self.fonts_db();
                let mut bare = w.clone();
                bare.set_axis(a.tag, f32::NAN);
                if let Some(f2) = face2
                    && let Some((m, _)) = db.var_values(f2, bare.weight, bare.italic, Some(bare.size as f32), &bare.axes)
                    && (m[k] - x).abs() < 0.001
                {
                    w.set_axis(a.tag, f32::NAN);
                }
            }
        }
    }
}
