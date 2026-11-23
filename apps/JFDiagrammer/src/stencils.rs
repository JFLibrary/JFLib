// stencils -- JD9: the stencils of the Shapes panel (FEATURES SL-01 .. SL-09, MS-04, MS-06,
// MS-07, LG-04): which are there, which are open, the favourites, the search
//
//   BUILTIN                     the stencils built from stencils/ by build.rs (PLAN.md V2): both
//                               .vssx of each (metric, US) embedded in the program
//   Stencils                    the panel's state: every stencil read (open ones, the built-in
//                               ones the search looks into), the open ones in panel order, the
//                               tile armed for click-placing, the panel widget's state
//   init(app)                   read the built-in stencils (the region's variant)
//   open_builtin / open_file / open_new / close   a stencil into the panel and out of it
//   add_favorite(app, ui, key, entry)   copy a master into "Favoriten" (data://shapes/
//                               favorites.vssx, created on first use; MS-06, SL-07)
//   search(app)                 the hits of the panel's query: NameU, the names in all five
//                               languages, the keywords, (JD15) the QuickInfos (SL-04), over
//                               every stencil read; every word, or one of them with the option
//                               "Shape-Suche: ein Wort genügt"
//   MORE_MENU / command(app, ui, id)   "Weitere Shapes" (SL-05): Meine Shapes, the categories
//                               with their stencils, Schablone öffnen, Neue Schablone (metric /
//                               US); the rows of each submenu are Mi::List rows, their commands
//                               come here
//   write_builtin(dir)          --write-stencils DIR: the embedded .vssx files into a folder (the
//                               user's check in Visio)
//
// A stencil is a .vssx document (diagram_io): the built-in ones, files the user opens and the
// own ones (favourites, new stencils) all go through vsdx::read and the same panel code.  Names
// come from the JFD_ cells (diagram_core::instance::MasterTexts) in the current language; a
// stencil of other programs shows its masters' Name.  The variant of a built-in stencil follows
// the region (LG-07: en-US = US units, every other region metric) when it is read.
// Headless runs (scripts) keep the favourites in memory, windows in data://shapes.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use diagram_core::instance::{self, MasterTexts, StencilTexts};
use diagram_core::{DocKind, Document, MasterRef};
use diagram_io::vsdx::{self, Measure};
use ui::i18n::{self, t, tf};
use ui::*;

use crate::Diagrammer;

/// a stencil built from stencils/ (build.rs writes the table)
pub(crate) struct Builtin {
    /// the output file stem ("standard")
    pub file: &'static str,
    /// the stencil's universal name ("Basic Shapes")
    pub stencil: &'static str,
    /// its category in "Weitere Shapes" (general, flowchart ...)
    pub category: &'static str,
    /// the metric and the US .vssx
    pub metric: &'static [u8],
    pub us: &'static [u8],
}

/// the built-in stencils
pub(crate) static BUILTIN: &[Builtin] = include!(concat!(env!("OUT_DIR"), "/stencils.rs"));

/// where a stencil comes from
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Origin {
    /// BUILTIN[k]
    Builtin(usize),
    /// a file the user opened
    File(String),
    /// the favourites (MS-06)
    Favorites,
    /// a new stencil, not saved yet (its file once saved)
    New(Option<String>),
}

/// a master shown in the panel
pub(crate) struct Entry {
    pub master: MasterRef,
    pub texts: MasterTexts,
}

/// a stencil read
pub(crate) struct Loaded {
    /// the panel's key (never reused)
    pub key: u32,
    pub origin: Origin,
    pub doc: Document,
    pub texts: StencilTexts,
    /// the shown masters in the stencil's order (hidden masters left out)
    pub entries: Vec<Entry>,
    /// the file name without extension: the title when the stencil has none of its own
    pub stem: String,
    /// takes masters (favourites, new stencils)
    pub editable: bool,
    /// changed since it was saved
    pub dirty: bool,
    /// the previews of its masters (stencils_panel.rs), made on first use; None after changes
    pub previews: Option<Rc<crate::stencils_panel::Previews>>,
    /// counts the changes (the tiles' version)
    pub serial: u32,
}

impl Loaded {
    /// the title in language `lang`
    pub fn title(&self, lang: usize) -> String {
        let n = self.texts.name(lang);
        if !n.is_empty() {
            return String::from(n);
        }
        match self.origin {
            Origin::Favorites => String::from(t("Favoriten")),
            _ => self.stem.clone(),
        }
    }

    pub(crate) fn rebuild_entries(&mut self) {
        let doc = &self.doc;
        let hidden = doc.sym("Hidden");
        self.entries = doc
            .masters()
            .filter(|&m| {
                let h = doc.master(m).map(|x| &x.header);
                !h.is_some_and(|h| hidden.and_then(|s| diagram_core::attr(&h.attrs, s)).is_some_and(|v| doc.str(v) == "1"))
            })
            .map(|m| Entry { master: m, texts: MasterTexts::of(doc, m) })
            .collect();
        self.previews = None;
        self.serial = self.serial.wrapping_add(1);
    }
}

/// the state of the Shapes panel
pub(crate) struct Stencils {
    /// the panel widget's state (active view, view mode, search text, splitter)
    pub panel: StencilState,
    /// every stencil read
    pub loaded: Vec<Loaded>,
    /// the keys of the open stencils in panel order
    pub open: Vec<u32>,
    /// click-placing: the armed tile (stencil key, entry index)
    pub armed: Option<(u32, usize)>,
    /// the tile whose context menu was opened last (its rows act on it)
    pub menu_tile: Option<(u32, usize)>,
    /// Schablone öffnen: the file dialog, its path
    pub open_dlg: Shared<i32>,
    pub open_path: String,
    /// the search's last query and its hits (stencil key, entry index)
    pub hits_for: String,
    pub hits: Vec<(u32, usize)>,
    /// a message for the panel (a stencil that could not be read)
    pub error: Option<String>,
    /// open the "Weitere Shapes" menu next pass (the command from the ribbon), where the
    /// panel's button was last (window px)
    pub more_menu: bool,
    pub more_at: Option<[i32; 2]>,
    next_key: u32,
}

impl Stencils {
    pub(crate) fn new() -> Stencils {
        Stencils {
            panel: StencilState::new(),
            loaded: Vec::new(),
            open: Vec::new(),
            armed: None,
            menu_tile: None,
            open_dlg: Shared::new(0),
            open_path: String::new(),
            hits_for: String::new(),
            hits: Vec::new(),
            error: None,
            more_menu: false,
            more_at: None,
            next_key: 1,
        }
    }

    /// the stencil with this key
    pub(crate) fn get(&self, key: u32) -> Option<&Loaded> {
        self.loaded.iter().find(|l| l.key == key)
    }

    pub(crate) fn get_mut(&mut self, key: u32) -> Option<&mut Loaded> {
        self.loaded.iter_mut().find(|l| l.key == key)
    }

    /// the stencil and master of a tile
    pub(crate) fn entry(&self, key: u32, k: usize) -> Option<(&Loaded, &Entry)> {
        let l = self.get(key)?;
        Some((l, l.entries.get(k)?))
    }

    /// the stencil read from `origin`, if it is
    fn find(&self, origin: &Origin) -> Option<u32> {
        self.loaded.iter().find(|l| &l.origin == origin).map(|l| l.key)
    }

    // a document as a stencil of the panel -> its key
    fn add(&mut self, origin: Origin, doc: Document, stem: &str, editable: bool) -> u32 {
        let key = self.next_key;
        self.next_key += 1;
        let texts = StencilTexts::of(&doc);
        let mut l = Loaded { key, origin, doc, texts, entries: Vec::new(), stem: String::from(stem), editable, dirty: false, previews: None, serial: 0 };
        l.rebuild_entries();
        self.loaded.push(l);
        key
    }

    /// show stencil `key` in the panel (at the end) and make it the active one
    pub(crate) fn show(&mut self, key: u32) {
        if !self.open.contains(&key) {
            self.open.push(key);
        }
        self.panel.active = key;
    }

    /// whether stencil `key` is open in the panel
    pub(crate) fn is_open(&self, key: u32) -> bool {
        self.open.contains(&key)
    }
}

/// the current language as an index into instance::LANGS (the catalogue's columns are the same)
pub(crate) fn lang() -> usize {
    (i18n::language() as usize).min(instance::LANGS.len() - 1)
}

/// the measuring system of the region (LG-07): en-US uses inches and Letter, every other
/// region millimetres and A4
pub(crate) fn measure(app: &Diagrammer) -> Measure {
    let choice = app.opts.get(&crate::options::REGION);
    let tag = match crate::lang::REGIONS.get(choice).copied().unwrap_or("") {
        "" => ["de-DE", "en-US", "es-ES", "it-IT", "fr-FR"][lang().min(4)],
        t => t,
    };
    if tag == "en-US" { Measure::Us } else { Measure::Metric }
}

/// read the built-in stencils (the region's variant): the search looks into them, the panel
/// shows them once opened
pub(crate) fn init(app: &mut Diagrammer) {
    for k in 0..BUILTIN.len() {
        load_builtin(app, k);
    }
}

// read BUILTIN[k] unless it is read -> its key
fn load_builtin(app: &mut Diagrammer, k: usize) -> Option<u32> {
    if let Some(key) = app.stencils.find(&Origin::Builtin(k)) {
        return Some(key);
    }
    let b = BUILTIN.get(k)?;
    let bytes = if measure(app) == Measure::Us { b.us } else { b.metric };
    match vsdx::read(bytes, &vsdx::Limits::VSDX) {
        Ok(l) => Some(app.stencils.add(Origin::Builtin(k), l.doc, b.file, false)),
        Err(e) => {
            app.stencils.error = Some(format!("{}: {e}", b.file));
            None
        }
    }
}

/// open built-in stencil k in the panel
pub(crate) fn open_builtin(app: &mut Diagrammer, ui: &mut Ui, k: usize) {
    if let Some(key) = load_builtin(app, k) {
        app.stencils.show(key);
        ui.repass();
    }
}

/// (JD13) open built-in stencil k in measure `m` (a template's units): the region's variant is the
/// stencil every other place uses; the other one is read as a stencil of its own, named like
/// its file ("jfd_flowchart_u.vssx") -> its key
pub(crate) fn open_builtin_in(app: &mut Diagrammer, ui: &mut Ui, k: usize, m: Measure) -> Option<u32> {
    if m == measure(app) {
        let key = load_builtin(app, k)?;
        app.stencils.show(key);
        ui.repass();
        return Some(key);
    }
    let b = BUILTIN.get(k)?;
    let name = diagram_io::stencil::file_name(b.file, m);
    let origin = Origin::File(name);
    let key = match app.stencils.find(&origin) {
        Some(key) => key,
        None => {
            let l = vsdx::read(if m == Measure::Us { b.us } else { b.metric }, &vsdx::Limits::VSDX).ok()?;
            app.stencils.add(origin, l.doc, b.file, false)
        }
    };
    app.stencils.show(key);
    ui.repass();
    Some(key)
}

/// open a stencil file in the panel (Schablone öffnen, Backstage › Öffnen of a .vssx)
pub(crate) fn open_file(app: &mut Diagrammer, ui: &mut Ui, path: &str) -> bool {
    let origin = Origin::File(String::from(path));
    if let Some(key) = app.stencils.find(&origin) {
        app.stencils.show(key);
        ui.repass();
        return true;
    }
    let name = plat::display_name(path);
    let stem = String::from(name.rsplit_once('.').map_or(name.as_str(), |x| x.0));
    let Some(bytes) = plat::read_file(path) else {
        app.stencils.error = Some(tf("„{}“ kann nicht geöffnet werden.", &[&name]));
        ui.repass();
        return false;
    };
    match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) if l.doc.kind == DocKind::Stencil => {
            let key = app.stencils.add(origin, l.doc, &stem, false);
            app.stencils.show(key);
            app.stencils.error = None;
            ui.recent_add(path);
        }
        Ok(_) => app.stencils.error = Some(tf("„{}“ ist keine Schablone.", &[&name])),
        Err(e) => app.stencils.error = Some(format!("{} {e}", tf("„{}“ kann nicht geöffnet werden.", &[&name]))),
    }
    ui.repass();
    true
}

/// Neue Schablone (metric / US): an empty stencil that takes masters (SL-05, MS-06)
pub(crate) fn open_new(app: &mut Diagrammer, ui: &mut Ui, m: Measure) {
    let n = app.stencils.loaded.iter().filter(|l| matches!(l.origin, Origin::New(_))).count() + 1;
    let stem = tf("Schablone{}", &[&format!("{n}")]);
    let doc = vsdx::new_document(DocKind::Stencil, m);
    let key = app.stencils.add(Origin::New(None), doc, &stem, true);
    app.stencils.show(key);
    ui.repass();
}

/// close stencil `key` (a changed own stencil is saved first)
pub(crate) fn close(app: &mut Diagrammer, ui: &mut Ui, key: u32) {
    if app.stencils.get(key).is_some_and(|l| l.dirty) {
        save(app, ui, key);
    }
    let st = &mut app.stencils;
    let Some(pos) = st.open.iter().position(|&k| k == key) else { return };
    st.open.remove(pos);
    if st.armed.is_some_and(|a| a.0 == key) {
        st.armed = None;
    }
    if st.panel.active == key {
        st.panel.active = st.open.get(pos.min(st.open.len().saturating_sub(1))).copied().unwrap_or(SK_QUICK);
    }
    // files and new stencils leave memory; the built-in ones stay for the search
    if let Some(i) = st.loaded.iter().position(|l| l.key == key) {
        if !matches!(st.loaded[i].origin, Origin::Builtin(_) | Origin::Favorites) {
            st.loaded.remove(i);
        }
    }
    ui.repass();
}

// ------------------------------------------------------------------ own stencils

/// the folder of the own stencils: data://shapes (None in headless runs: in memory only)
fn shapes_dir(ui: &Ui) -> Option<String> {
    if ui.headless() {
        return None;
    }
    let dir = String::from("data://shapes");
    plat::mkdir(&dir);
    Some(dir)
}

/// the favourites stencil: read from data://shapes/favorites.vssx or made new -> its key
pub(crate) fn favorites(app: &mut Diagrammer, ui: &Ui) -> u32 {
    if let Some(key) = app.stencils.find(&Origin::Favorites) {
        return key;
    }
    let path = shapes_dir(ui).map(|d| format!("{d}/favorites.vssx"));
    let doc = path
        .as_deref()
        .and_then(plat::read_file)
        .and_then(|b| vsdx::read(&b, &vsdx::Limits::VSDX).ok())
        .map(|l| l.doc)
        .filter(|d| d.kind == DocKind::Stencil)
        .unwrap_or_else(|| new_favorites(measure(app)));
    app.stencils.add(Origin::Favorites, doc, "favorites", true)
}

// an empty favourites stencil with its title in five languages
fn new_favorites(m: Measure) -> Document {
    let mut doc = vsdx::new_document(DocKind::Stencil, m);
    let title = instance::join_fields(&["Favoriten", "Favorites", "Favoritos", "Preferiti", "Favoris"]);
    let mut tx = doc.transaction("Favoriten");
    let sym = tx.intern(&title);
    let _ = tx.set_user(diagram_core::SheetRef::Document, instance::JFD_NAME, diagram_core::Value::Text(sym), Some(diagram_core::Unit::Str));
    tx.commit();
    doc
}

/// Zu meinen Shapes hinzufügen: copy the master of tile (key, k) into the favourites, open them
pub(crate) fn add_favorite(app: &mut Diagrammer, ui: &mut Ui, key: u32, k: usize) {
    let fav = favorites(app, ui);
    add_master_to(app, ui, key, k, fav, None, None);
    app.stencils.show(fav);
}

/// copy the master of tile (key, k) into the own stencil `target` (before entry `before`;
/// None: at the end; quick: into the quick part or out of it, MS-04) and save it; a tile of the
/// same stencil moves
pub(crate) fn add_master_to(app: &mut Diagrammer, ui: &mut Ui, key: u32, k: usize, target: u32, before: Option<usize>, quick: Option<bool>) {
    let st = &mut app.stencils;
    let Some(ti) = st.loaded.iter().position(|l| l.key == target) else { return };
    let Some(si) = st.loaded.iter().position(|l| l.key == key) else { return };
    if !st.loaded[ti].editable {
        return;
    }
    // the source stencil's document is borrowed while the target changes: a copy of it when
    // both are the same (reordering inside a stencil goes through move_master instead)
    if si == ti {
        if let (Some(b), Some(m)) = (before, st.loaded[ti].entries.get(k).map(|e| e.master)) {
            let l = &mut st.loaded[ti];
            let mut tx = l.doc.transaction(t("Shape verschieben"));
            let moved = instance::move_master(&mut tx, m, b).is_ok();
            let flagged = quick.is_some_and(|q| set_quick(&mut tx, m, q));
            if moved || flagged {
                tx.commit();
                l.rebuild_entries();
                l.dirty = true;
            }
        }
        save(app, ui, target);
        return;
    }
    let Some(m) = st.loaded[si].entries.get(k).map(|e| e.master) else { return };
    let (src, dst) = if si < ti {
        let (a, b) = st.loaded.split_at_mut(ti);
        (&a[si], &mut b[0])
    } else {
        let (a, b) = st.loaded.split_at_mut(si);
        (&b[0], &mut a[ti])
    };
    // the same master (UniqueID) is there already: nothing to add (MS-02)
    let mut tx = dst.doc.transaction(t("Zu meinen Shapes hinzufügen"));
    let before_n = tx.doc().masters().count();
    match instance::import_master(&mut tx, &src.doc, m) {
        Ok(new) => {
            let added = tx.doc().masters().count() > before_n;
            if added {
                if let Some(b) = before {
                    let _ = instance::move_master(&mut tx, new, b);
                }
                if let Some(q) = quick {
                    set_quick(&mut tx, new, q);
                }
            }
            tx.commit();
            if added {
                dst.rebuild_entries();
                dst.dirty = true;
            }
        }
        Err(_) => tx.rollback(),
    }
    save(app, ui, target);
}

// a master into the quick part of its stencil or out of it (User.JFD_Quick) -> changed
fn set_quick(tx: &mut diagram_core::Transaction, m: MasterRef, quick: bool) -> bool {
    if MasterTexts::of(tx.doc(), m).quick == quick {
        return false;
    }
    let v = tx.intern(if quick { "1" } else { "0" });
    tx.set_user(diagram_core::SheetRef::Master(m), instance::JFD_QUICK, diagram_core::Value::Text(v), Some(diagram_core::Unit::Str)).is_ok()
}

/// remove entry k from the own stencil `key`
pub(crate) fn remove_master(app: &mut Diagrammer, ui: &mut Ui, key: u32, k: usize) {
    {
        let Some(l) = app.stencils.get_mut(key) else { return };
        if !l.editable {
            return;
        }
        let Some(m) = l.entries.get(k).map(|e| e.master) else { return };
        let mut tx = l.doc.transaction(t("Aus Schablone entfernen"));
        if tx.delete_master(m).is_ok() {
            tx.commit();
            l.rebuild_entries();
            l.dirty = true;
        }
    }
    if app.stencils.armed.is_some_and(|a| a.0 == key) {
        app.stencils.armed = None;
    }
    save(app, ui, key);
}

/// save an own stencil: the favourites to data://shapes/favorites.vssx, a new stencil to
/// data://shapes/<name>.vssx (headless: nothing is written)
pub(crate) fn save(app: &mut Diagrammer, ui: &mut Ui, key: u32) {
    let dir = shapes_dir(ui);
    let Some(l) = app.stencils.get_mut(key) else { return };
    if !l.editable {
        return;
    }
    let path = match (&l.origin, &dir) {
        (_, None) => {
            l.dirty = false;
            return;
        }
        (Origin::Favorites, Some(d)) => format!("{d}/favorites.vssx"),
        (Origin::New(Some(p)), _) => p.clone(),
        (Origin::New(None), Some(d)) => format!("{d}/{}.vssx", l.stem),
        (Origin::File(p), _) => p.clone(),
        (Origin::Builtin(_), _) => return,
    };
    match vsdx::write(&l.doc, &vsdx::WriteOptions::DEFAULT) {
        Ok(bytes) if plat::write_file(&path, &bytes) => {
            l.dirty = false;
            if let Origin::New(p) = &mut l.origin {
                *p = Some(path);
            }
        }
        _ => app.stencils.error = Some(tf("„{}“ lässt sich nicht speichern.", &[&l.title(lang())])),
    }
    ui.repass();
}

/// the own stencils in data://shapes other than the favourites (Weitere Shapes › Meine Shapes)
pub(crate) fn my_files(ui: &Ui) -> Vec<String> {
    let Some(dir) = shapes_dir(ui) else { return Vec::new() };
    let mut v: Vec<String> = Vec::new();
    plat::dir_list(&dir, &mut |name, is_dir, _| {
        let l = name.to_ascii_lowercase();
        if !is_dir && (l.ends_with(".vssx") || l.ends_with(".vssm")) && l != "favorites.vssx" {
            v.push(format!("{dir}/{name}"));
        }
    });
    v.sort();
    v
}

// ------------------------------------------------------------------ search

/// the hits of the panel's query (cached while it stays the same)
pub(crate) fn search(app: &mut Diagrammer) -> &[(u32, usize)] {
    // (JD15, SL-04) every word or one of them (Optionen › Erweitert); the cache knows which
    let any = app.opts.get(&crate::options::SEARCH_ANY);
    let st = &mut app.stencils;
    let words_of = st.panel.query.trim().to_string();
    let q = if any { format!("\u{1}{words_of}") } else { words_of.clone() };
    if q != st.hits_for {
        st.hits.clear();
        let words: Vec<String> = words_of.split_whitespace().map(instance::fold).collect();
        let words: Vec<&str> = words.iter().map(|s| s.as_str()).collect();
        if !words.is_empty() {
            // the open stencils first, in panel order, then the others read
            let mut order: Vec<usize> = st.open.iter().filter_map(|k| st.loaded.iter().position(|l| l.key == *k)).collect();
            order.extend((0..st.loaded.len()).filter(|i| !st.open.contains(&st.loaded[*i].key)));
            for i in order {
                let l = &st.loaded[i];
                for (k, e) in l.entries.iter().enumerate() {
                    if e.texts.matches(&words, any) {
                        st.hits.push((l.key, k));
                    }
                }
            }
        }
        st.hits_for = q;
    }
    &app.stencils.hits
}

// ------------------------------------------------------------------ Weitere Shapes

/// the base ids of the submenus' rows (row k: base + k)
pub(crate) const ML_MINE: u32 = 0x7100;
const ML_CATEGORY: u32 = 0x7140; // + 0x40 per category
const ML_ROWS: u16 = 48;

/// the categories of "Weitere Shapes" (FEATURES 4.1) by their key in stencil.txt, in the order
/// of MORE_MENU's submenus (their labels are there)
pub(crate) static CATEGORIES: [&str; 9] = ["general", "flowchart", "business", "engineering", "network", "maps", "software", "schedule", "extras"];

// i18n
pub(crate) static MORE_MENU: [Mi; 15] = [
    Mi::Sub("Meine Shapes", &[Mi::List(ML_MINE, ML_ROWS, "")]),
    Mi::Sep,
    Mi::Sub("Allgemein", &[Mi::List(ML_CATEGORY, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sub("Flussdiagramm", &[Mi::List(ML_CATEGORY + 0x40, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sub("Geschäft", &[Mi::List(ML_CATEGORY + 0x80, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sub("Konstruktion", &[Mi::List(ML_CATEGORY + 0xC0, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sub("Netzwerk", &[Mi::List(ML_CATEGORY + 0x100, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sub("Pläne und Grundrisse", &[Mi::List(ML_CATEGORY + 0x140, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sub("Software und Datenbank", &[Mi::List(ML_CATEGORY + 0x180, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sub("Terminplan", &[Mi::List(ML_CATEGORY + 0x1C0, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sub("Extras", &[Mi::List(ML_CATEGORY + 0x200, ML_ROWS, "Folgt in einer späteren Version")]),
    Mi::Sep,
    Mi::Cmd(crate::commands::STENCIL_OPEN),
    Mi::Cmd(crate::commands::STENCIL_NEW_METRIC),
    Mi::Cmd(crate::commands::STENCIL_NEW_US),
];

// the built-in stencils of category c (indices into BUILTIN)
fn of_category(c: usize) -> Vec<usize> {
    let key = CATEGORIES[c];
    (0..BUILTIN.len()).filter(|&k| BUILTIN[k].category == key).collect()
}

/// every pass: the rows of the submenus (their titles in the current language, checked while
/// open)
pub(crate) fn menu_rows(app: &mut Diagrammer, ui: &mut Ui) {
    let l = lang();
    // Meine Shapes: Favoriten, then the own stencil files
    let mut mine: Vec<String> = alloc::vec![String::from(t("Favoriten"))];
    let files = my_files(ui);
    mine.extend(files.iter().map(|p| {
        let n = plat::display_name(p);
        String::from(n.rsplit_once('.').map_or(n.as_str(), |x| x.0))
    }));
    let fav_open = app.stencils.find(&Origin::Favorites).is_some_and(|k| app.stencils.is_open(k));
    ui.cmd_state(ML_MINE, if fav_open { CF_CHECKED } else { 0 });
    for (k, p) in files.iter().enumerate() {
        let open = app.stencils.find(&Origin::File(p.clone())).is_some_and(|k| app.stencils.is_open(k));
        ui.cmd_state(ML_MINE + 1 + k as u32, if open { CF_CHECKED } else { 0 });
    }
    ui.menu_list(ML_MINE, &mine);
    for c in 0..CATEGORIES.len() {
        let base = ML_CATEGORY + 0x40 * c as u32;
        let ks = of_category(c);
        let rows: Vec<String> = ks
            .iter()
            .map(|&k| app.stencils.find(&Origin::Builtin(k)).and_then(|key| app.stencils.get(key)).map_or_else(|| String::from(BUILTIN[k].stencil), |s| s.title(l)))
            .collect();
        for (i, &k) in ks.iter().enumerate() {
            let open = app.stencils.find(&Origin::Builtin(k)).is_some_and(|key| app.stencils.is_open(key));
            ui.cmd_state(base + i as u32, if open { CF_CHECKED } else { 0 });
        }
        ui.menu_list(base, &rows);
    }
}

/// App::command for the rows of "Weitere Shapes" and of a tile's menu -> handled
pub(crate) fn command(app: &mut Diagrammer, ui: &mut Ui, id: u32) -> bool {
    use crate::stencils_panel::{MI_ADD_FAVORITE, MI_REMOVE_MASTER};
    if id == MI_ADD_FAVORITE || id == MI_REMOVE_MASTER {
        if let Some((key, k)) = app.stencils.menu_tile.take() {
            if id == MI_ADD_FAVORITE {
                add_favorite(app, ui, key, k);
            } else {
                remove_master(app, ui, key, k);
            }
        }
        return true;
    }
    if (ML_MINE..ML_MINE + ML_ROWS as u32).contains(&id) {
        let k = (id - ML_MINE) as usize;
        if k == 0 {
            let fav = favorites(app, ui);
            toggle(app, ui, fav);
        } else if let Some(p) = my_files(ui).get(k - 1) {
            match app.stencils.find(&Origin::File(p.clone())) {
                Some(key) => toggle(app, ui, key),
                None => {
                    open_file(app, ui, &p.clone());
                }
            }
        }
        return true;
    }
    if (ML_CATEGORY..ML_CATEGORY + 0x40 * CATEGORIES.len() as u32).contains(&id) {
        let c = ((id - ML_CATEGORY) / 0x40) as usize;
        let i = ((id - ML_CATEGORY) % 0x40) as usize;
        if let Some(&k) = of_category(c).get(i) {
            match app.stencils.find(&Origin::Builtin(k)) {
                Some(key) if app.stencils.is_open(key) => close(app, ui, key),
                _ => open_builtin(app, ui, k),
            }
        }
        return true;
    }
    false
}

// open a stencil that is closed, close one that is open (a checked row of the menu)
fn toggle(app: &mut Diagrammer, ui: &mut Ui, key: u32) {
    if app.stencils.is_open(key) {
        close(app, ui, key);
    } else {
        app.stencils.show(key);
        ui.repass();
    }
}

/// --write-stencils DIR: the embedded .vssx files into DIR -> the paths written
pub(crate) fn write_builtin(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    for b in BUILTIN {
        for (m, bytes) in [(Measure::Metric, b.metric), (Measure::Us, b.us)] {
            let p = format!("{dir}/{}", diagram_io::stencil::file_name(b.file, m));
            if plat::write_file(&p, bytes) {
                out.push(p);
            }
        }
    }
    out
}
