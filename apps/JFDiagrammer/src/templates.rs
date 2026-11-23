// templates -- (JD13) the templates of Datei › Neu (FEATURES VL-01 .. VL-04, VL-10, VL-11, VL-20,
// VL-21, EX-11, DF-05): the built-in ones and the user's own
//
//   BUILTIN                     every form of every built-in template: build_steps.rs builds
//                               templates/<folder>/ into .vstx files (metric and US) and lists
//                               them (PLAN.md V2; the format: templates/README.md)
//   Catalog / Entry / Form      the catalogue: a template with its forms (the blank one, the
//                               starter diagrams), read once from the embedded files; the own
//                               templates are read again whenever Neu opens
//   catalog(app, ui)            the entries in catalogue order (built-in by order, own by name)
//   create(app, ui, entry, form)  a new, unnamed drawing from a form in the units chosen
//   create_from(app, ui, doc, dir)  ... from a template document (an own .vstx, Backstage ›
//                               Öffnen, the command line: DF-05, VL-03)
//   own_dir(app, ui)            the folder of the own templates: the option Speicherort für
//                               persönliche Vorlagen, else data://templates (--template-dir DIR
//                               in scripts)
//   save_as(app, ui)            Speichern unter › Als Vorlage: the save dialog in that folder
//   windows_of_panel(app)       the open stencils as the template's stencil windows (EX-11)
//   has_register(app, "cff")    a register the drawing's template brought (VL-04)
//   write_builtin(dir)          --write-templates DIR: every embedded .vstx into a folder
//
// A template is a .vstx document (diagram_io::template): a new drawing from it keeps its pages,
// masters, styles, design, stencil windows and rule set; to_drawing puts the page names and the
// texts into the language of the user and drops the catalogue's cells.  The stencils it names
// open in the Shapes panel in its order, the active one unfolded: built-in stencils by their
// file name ("jfd_flowchart_m.vssx", the US form "_u"; diagram_io::stencil::file_name), other
// names as paths.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{CellAddr, DocKind, Document, SheetRef};
use diagram_io::template::{self, StencilWindow};
use diagram_io::vsdx::{self, Format};
use gfx::Surface;
use ui::i18n::{t, tf};
use ui::*;

use crate::stencils::{self, Origin};
use crate::Diagrammer;

/// one form of a built-in template (build_steps.rs writes the table)
pub(crate) struct TemplateFile {
    /// the template's universal name ("Basic Flowchart")
    pub template: &'static str,
    /// the output file stem ("flowchart", "flowchart-sample")
    pub file: &'static str,
    /// the starter diagram's universal name (None: the blank form)
    pub starter: Option<&'static str>,
    /// its category (stencils::CATEGORIES)
    pub category: &'static str,
    /// its place in the catalogue (smaller first)
    pub order: u32,
    /// the metric and the US .vstx
    pub metric: &'static [u8],
    pub us: &'static [u8],
}

/// the built-in templates, every form
pub(crate) static BUILTIN: &[TemplateFile] = include!(concat!(env!("OUT_DIR"), "/templates.rs"));

/// the category key of the own templates
pub(crate) const MINE: &str = "mine";

/// where a form comes from
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Source {
    /// BUILTIN[k]
    Builtin(usize),
    /// an own template file
    File(String),
}

/// a form of a template: the blank one or a starter diagram
pub(crate) struct Form {
    pub source: Source,
    /// the starter diagram's names (None: the blank form, "Leer")
    pub starter: Option<[String; 5]>,
    /// its own description (else the template's)
    pub description: Option<[String; 5]>,
    /// the width : height of its first page, metric and US
    pub aspect: [f64; 2],
    /// the pictures made so far: (measure is US, width, height) and the page rendered
    thumbs: Vec<((bool, i32, i32), Rc<Surface>)>,
}

/// a template of the catalogue
pub(crate) struct Entry {
    /// its universal name (built-in) or its path (own)
    pub key: String,
    pub category: String,
    pub order: u32,
    /// its titles and description in five languages (own templates: none, the file's name)
    pub names: Option<[String; 5]>,
    pub description: Option<[String; 5]>,
    /// the file's name without extension (own templates)
    pub stem: String,
    pub forms: Vec<Form>,
    /// the form the catalogue chooses first
    pub default: usize,
}

impl Entry {
    /// its title in language `l`
    pub(crate) fn title(&self, l: usize) -> String {
        match &self.names {
            Some(n) if !n[l].is_empty() => n[l].clone(),
            _ => self.stem.clone(),
        }
    }

    /// the description of form `f` in language `l` ("" when it has none)
    pub(crate) fn about(&self, f: usize, l: usize) -> String {
        let own = self.forms.get(f).and_then(|x| x.description.as_ref());
        own.or(self.description.as_ref()).map(|d| d[l].clone()).unwrap_or_default()
    }

    /// whether it is an own template (a file of the user's folder)
    pub(crate) fn own(&self) -> bool {
        self.category == MINE
    }
}

/// the catalogue's state
#[derive(Default)]
pub(crate) struct Catalog {
    /// the built-in entries (read on first use) and the own ones (read when Neu opens)
    pub builtin: Vec<Entry>,
    pub own: Vec<Entry>,
    /// the own folder was read for this opening of Neu
    pub own_read: bool,
    /// the category shown: None every template, else a key of stencils::CATEGORIES or MINE
    pub category: Option<String>,
    /// the form chosen in the preview
    pub form: usize,
    /// (JD15, UI-05) the search over the templates' names (every language)
    pub query: String,
    /// (JD15, UI-04) the tile clicked last: entry, time, pointer (the double click's second
    /// press lands on the preview that opened)
    pub clicked: Option<(usize, u32, (i32, i32))>,
}

impl Catalog {
    /// entry `k` of the list catalog() gives (built-in first, then the own ones)
    pub(crate) fn get(&self, k: usize) -> Option<&Entry> {
        if k < self.builtin.len() { self.builtin.get(k) } else { self.own.get(k - self.builtin.len()) }
    }

    fn get_mut(&mut self, k: usize) -> Option<&mut Entry> {
        let n = self.builtin.len();
        if k < n { self.builtin.get_mut(k) } else { self.own.get_mut(k - n) }
    }

    /// how many entries there are
    pub(crate) fn len(&self) -> usize {
        self.builtin.len() + self.own.len()
    }
}

// ------------------------------------------------------------------ reading

/// the five texts of a JFD_ cell's value, None when it has none
fn five(v: &Option<[String; 5]>) -> Option<[String; 5]> {
    v.as_ref().filter(|x| x.iter().any(|s| !s.is_empty())).cloned()
}

// the built-in entries, grouped by template, in catalogue order
fn read_builtin() -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for (k, f) in BUILTIN.iter().enumerate() {
        let Ok(l) = vsdx::read(f.metric, &vsdx::Limits::VSDX) else { continue };
        let info = template::info(&l.doc);
        let us = vsdx::read(f.us, &vsdx::Limits::VSDX).map(|u| aspect_of(&u.doc)).unwrap_or(A4);
        let form = Form { source: Source::Builtin(k), starter: five(&info.starter), description: None, aspect: [aspect_of(&l.doc), us], thumbs: Vec::new() };
        let i = match out.iter().position(|e| e.key == f.template) {
            Some(i) => i,
            None => {
                out.push(Entry {
                    key: String::from(f.template),
                    category: String::from(f.category),
                    order: f.order,
                    names: five(&info.name),
                    description: five(&info.description),
                    stem: String::from(f.file),
                    forms: Vec::new(),
                    default: 0,
                });
                out.len() - 1
            }
        };
        let e = &mut out[i];
        if f.starter.is_none() {
            // the blank form carries the template's texts
            e.names = five(&info.name).or(e.names.take());
            e.description = five(&info.description).or(e.description.take());
        }
        // a starter diagram with a description of its own
        let own = five(&info.description).filter(|d| f.starter.is_some() && Some(d) != e.description.as_ref());
        let form = Form { description: own, ..form };
        if info.default {
            e.default = e.forms.len();
        }
        e.forms.push(form);
    }
    out.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.key.cmp(&b.key)));
    out
}

/// the folder of the own templates (VL-03): the option, else data://templates; scripts name one
/// with --template-dir (None without: headless runs keep nothing)
pub(crate) fn own_dir(app: &Diagrammer, ui: &Ui) -> Option<String> {
    if let Some(d) = plat::arg_value("--template-dir") {
        let d = if d.starts_with('/') || d.get(1..2) == Some(":") { d } else { format!("{}/{}", plat::cwd(), d) };
        plat::mkdir(&d);
        return Some(d);
    }
    let opt = app.opts.get(&crate::options::TEMPLATES);
    if !opt.trim().is_empty() {
        return Some(String::from(opt.trim()));
    }
    if ui.headless() {
        return None;
    }
    let dir = String::from("data://templates");
    plat::mkdir(&dir);
    Some(dir)
}

// the own templates: the .vstx / .vstm files of the folder, by name
fn read_own(dir: &str) -> Vec<Entry> {
    let mut files: Vec<String> = Vec::new();
    plat::dir_list(dir, &mut |name, is_dir, _| {
        let l = name.to_ascii_lowercase();
        if !is_dir && (l.ends_with(".vstx") || l.ends_with(".vstm")) {
            files.push(String::from(name));
        }
    });
    files.sort_by_key(|a| a.to_lowercase());
    files
        .into_iter()
        .map(|name| {
            let path = format!("{dir}/{name}");
            let stem = String::from(name.rsplit_once('.').map_or(name.as_str(), |x| x.0));
            let a = plat::read_file(&path).and_then(|b| vsdx::read(&b, &vsdx::Limits::VSDX).ok()).map_or(A4, |l| aspect_of(&l.doc));
            Entry {
                key: path.clone(),
                category: String::from(MINE),
                order: 0,
                names: None,
                description: None,
                stem,
                forms: alloc::vec![Form { source: Source::File(path), starter: None, description: None, aspect: [a, a], thumbs: Vec::new() }],
                default: 0,
            }
        })
        .collect()
}

/// the catalogue, read as needed: the built-in templates once, the own ones once per opening
/// of Neu (Catalog::own_read is cleared when the File register opens)
pub(crate) fn catalog(app: &mut Diagrammer, ui: &Ui) {
    if app.catalog.builtin.is_empty() && !BUILTIN.is_empty() {
        app.catalog.builtin = read_builtin();
    }
    if !app.catalog.own_read {
        app.catalog.own_read = true;
        app.catalog.own = own_dir(app, ui).map(|d| read_own(&d)).unwrap_or_default();
    }
}

// the template document of a form in the chosen units (an own template has its own)
fn form_doc(f: &Form, us: bool) -> Option<Document> {
    let bytes: Vec<u8>;
    let data: &[u8] = match &f.source {
        Source::Builtin(k) => {
            let b = BUILTIN.get(*k)?;
            if us { b.us } else { b.metric }
        }
        Source::File(p) => {
            bytes = plat::read_file(p)?;
            &bytes
        }
    };
    vsdx::read(data, &vsdx::Limits::VSDX).ok().map(|l| l.doc)
}

/// the picture of form `f` of entry `k`: its first page rendered into w x h pixels (kept)
pub(crate) fn thumb(app: &mut Diagrammer, k: usize, f: usize, w: i32, h: i32) -> Option<Rc<Surface>> {
    let us = app.units_us == 1;
    let mut fonts = app.fonts.clone();
    let e = app.catalog.get_mut(k)?;
    let us = us && !e.own();
    let form = e.forms.get_mut(f)?;
    if let Some((_, s)) = form.thumbs.iter().find(|(key, _)| *key == (us, w, h)) {
        return Some(s.clone());
    }
    let doc = form_doc(form, us)?;
    let page = doc.pages().find(|&p| doc.page(p).is_some_and(|pg| !pg.header.background))?;
    let mut v = diagram_view::PageView::new(&doc, page, &mut fonts, &diagram_core::render::Options::default());
    let sf = Rc::new(v.render(w.max(8) as u32, h.max(8) as u32));
    if form.thumbs.len() > 8 {
        form.thumbs.remove(0);
    }
    form.thumbs.push(((us, w, h), sf.clone()));
    Some(sf)
}

/// the width : height of the first page of form `f` of entry `k` in the units chosen
pub(crate) fn aspect(app: &Diagrammer, k: usize, f: usize) -> f64 {
    let us = app.units_us == 1;
    app.catalog.get(k).and_then(|e| e.forms.get(f)).map_or(A4, |x| x.aspect[us as usize])
}

// landscape A4, the aspect of a page whose size is unknown
const A4: f64 = 297.0 / 210.0;

fn aspect_of(doc: &Document) -> f64 {
    let (w, h) = page_size(doc);
    if w > 0.0 && h > 0.0 { w / h } else { A4 }
}

// the size of the first foreground page (inches)
fn page_size(doc: &Document) -> (f64, f64) {
    let Some(p) = doc.pages().find(|&p| doc.page(p).is_some_and(|pg| !pg.header.background)) else { return (0.0, 0.0) };
    let num = |name: &str| -> f64 {
        let Some(sym) = doc.sym(name) else { return 0.0 };
        let r = doc.resolve(SheetRef::Page(p), &CellAddr::single(sym));
        r.cached.or(r.cell).and_then(|c| c.value.num()).unwrap_or(0.0)
    };
    (num("PageWidth"), num("PageHeight"))
}

// ------------------------------------------------------------------ a new drawing

/// Erstellen: a new drawing from form `f` of entry `k` in the units chosen
pub(crate) fn create(app: &mut Diagrammer, ui: &mut Ui, k: usize, f: usize) -> bool {
    // (JD15, UI-12) the changes of the open drawing first
    if !crate::ask_save::guard(app, ui, crate::ask_save::Next::Template(k, f)) {
        return false;
    }
    let us = app.units_us == 1;
    let Some(e) = app.catalog.get(k) else { return false };
    let own = e.own();
    let Some(form) = e.forms.get(f) else { return false };
    let dir = match &form.source {
        Source::File(p) => p.rsplit_once(['/', '\\']).map(|x| String::from(x.0)),
        Source::Builtin(_) => None,
    };
    let Some(doc) = form_doc(form, us && !own) else {
        app.error = Some((tf("„{}“ kann nicht geöffnet werden.", &[&e.title(stencils::lang())]), String::from(t("Die Datei lässt sich nicht lesen."))));
        ui.repass();
        return false;
    };
    app.template = None;
    create_from(app, ui, doc, dir.as_deref());
    true
}

/// a new, unnamed drawing from a template document (DF-05); `dir` is the template's folder, where
/// stencils it names by a bare file name are looked for too
pub(crate) fn create_from(app: &mut Diagrammer, ui: &mut Ui, mut doc: Document, dir: Option<&str>) {
    let windows = template::stencil_windows(&doc);
    template::to_drawing(&mut doc, stencils::lang());
    let name = crate::edit_doc::next_name();
    // (JD16, DF-05) a drawing from a .vstm keeps its macro project: a .vsdm
    let format = Format { kind: DocKind::Drawing, macros: doc.macros };
    crate::edit_doc::install(app, ui, doc, name, None, format, None);
    open_stencils(app, ui, &windows, dir);
}

// the stencils of a template into the Shapes panel, in its order, the active one unfolded; the
// stencils open before are closed (the panel belongs to the drawing, as Visio's window does)
fn open_stencils(app: &mut Diagrammer, ui: &mut Ui, list: &[StencilWindow], dir: Option<&str>) {
    for key in app.stencils.open.clone() {
        stencils::close(app, ui, key);
    }
    let mut active = None;
    let mut missing: Vec<String> = Vec::new();
    for w in list {
        let name = String::from(w.document.rsplit(['/', '\\']).next().unwrap_or(&w.document));
        // a built-in stencil by its file name ("jfd_flowchart_u.vssx"), in the measure the name
        // says; Visio's own stencils (BASIC_M.VSSX ...) are none of ours
        let builtin = diagram_io::stencil::parse_file_name(&name)
            .and_then(|(stem, m)| stencils::BUILTIN.iter().position(|b| b.file.eq_ignore_ascii_case(&stem)).map(|k| (k, m)));
        let key = if let Some((b, m)) = builtin {
            stencils::open_builtin_in(app, ui, b, m)
        } else {
            let mut tries: Vec<String> = alloc::vec![w.document.clone()];
            if let Some(d) = dir {
                tries.push(format!("{d}/{name}"));
            }
            tries.push(format!("data://shapes/{name}"));
            tries.into_iter().find(|p| plat::read_file(p).is_some()).and_then(|p| {
                stencils::open_file(app, ui, &p);
                app.stencils.loaded.iter().find(|l| l.origin == Origin::File(p.clone())).map(|l| l.key)
            })
        };
        match key {
            Some(k) if w.active => active = Some(k),
            Some(_) => {}
            None => missing.push(name),
        }
    }
    if let Some(k) = active.or_else(|| app.stencils.open.first().copied()) {
        app.stencils.panel.active = k;
    }
    if !missing.is_empty() {
        if let Some(o) = app.opened.as_mut() {
            o.notice = Some((String::from(t("Hinweise zur Vorlage")), tf("Diese Schablonen der Vorlage wurden nicht gefunden: {}", &[&missing.join(", ")])));
        }
    }
    ui.repass();
}

// ------------------------------------------------------------------ own templates (EX-11)

/// the open stencils as stencil windows of a template: built-in ones by their file name, files
/// by their path; new stencils not saved yet and the favourites are left out
pub(crate) fn windows_of_panel(app: &Diagrammer) -> Vec<StencilWindow> {
    let region = stencils::measure(app);
    let active = app.stencils.panel.active;
    app.stencils
        .open
        .iter()
        .filter_map(|&key| {
            let l = app.stencils.get(key)?;
            let document = match &l.origin {
                Origin::Builtin(k) => diagram_io::stencil::file_name(stencils::BUILTIN.get(*k)?.file, region),
                Origin::File(p) | Origin::New(Some(p)) => p.clone(),
                Origin::Favorites | Origin::New(None) => return None,
            };
            Some(StencilWindow { document, active: key == active })
        })
        .collect()
}

/// Speichern unter › Als Vorlage (EX-11): the save dialog in the own template folder with the
/// drawing's name as a .vstx
pub(crate) fn save_as(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(o) = app.opened.as_ref() else { return };
    let stem = o.name.clone();
    app.save_path = match own_dir(app, ui) {
        Some(d) => format!("{d}/{stem}.vstx"),
        None => format!("{stem}.vstx"),
    };
    app.backstage = None;
    app.catalog.own_read = false;
    app.save_dlg.set(1);
    ui.repass();
}

/// whether the drawing's template brought register `name` (VL-04: User.JFD_Registers)
pub(crate) fn has_register(app: &Diagrammer, name: &str) -> bool {
    app.opened.as_ref().is_some_and(|o| template::registers(&o.doc).iter().any(|r| r.eq_ignore_ascii_case(name)))
}

/// --write-templates DIR: every embedded .vstx into a folder -> the files written
pub(crate) fn write_builtin(dir: &str) -> Vec<String> {
    let mut written = Vec::new();
    plat::mkdir(dir);
    for b in BUILTIN {
        for (suffix, bytes) in [("m", b.metric), ("u", b.us)] {
            let path = format!("{dir}/{}_{suffix}.vstx", b.file);
            if plat::write_file(&path, bytes) {
                written.push(path);
            }
        }
    }
    written
}
