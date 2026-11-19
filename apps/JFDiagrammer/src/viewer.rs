// viewer -- JD4, milestone "Betrachter": a drawing opened and shown page by page; since JD7
// editable (edit_doc.rs, edit_canvas.rs)
//
//   Opened                       the document (diagram_io::vsdx::read or a new one), its pages
//                                (foreground pages first, then the background pages, each a tab),
//                                the notice shown under the ribbon, its edit state (edit_doc.rs)
//   open(app, ui, path)          read a .vsdx/.vssx/.vstx (and the macro variants) from the file
//                                dialog of Backstage › Öffnen or the command line; a notice names
//                                substituted fonts (TX-09), macros (kept, never run, DF-04) and
//                                what the reader warned about; (JD9) a .vssx goes to the Shapes
//                                panel instead
//   close(app, ui)               back to the empty drawing
//   paint(app, k) -> PagePaint   page k as a ui.diagram content (DgPaint): built on first use,
//                                kept while the document is open, brought up to date after edits
//                                (sync: the changed top-level shapes again, else the page anew)
//   model(app, k)                the canvas' model: one node, the page's sheet (fit, overview)
//   page_width(app)              zoom to the page's width (Ansicht › Seitenbreite)
//   fonts(ui)                    the fonts pages are drawn with: the style's Inter (sans) and
//                                the installed fonts the drawing names (text::fontdb); what is
//                                not installed is substituted by class (diagram_view::fonts)
//
// Coordinates: the canvas' world unit is a DIP at 100 % (ui.diagram), a paper inch is 96 of
// them, y down from the page's top left corner; diagram_view draws in paper inches with y up.
// The paint draws the page's sheet with its shadow under the grid (paint_under) and the page's
// display list over it (paint, render2d's CPU path through gfx; ui.diagram keeps the result in
// its backing store and pans with tiles).

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use diagram_core::render::Options;
use diagram_core::{DocKind, Document, PageRef};
use diagram_io::vsdx;
use diagram_view::{Class, FontSet, FontSource, PageView};
use render2d::Camera;
use ui::i18n::{t, tf};
use ui::*;

use crate::Diagrammer;

/// DIP per paper inch at 100 %
pub(crate) const DIP: f64 = 96.0;

/// a tab of the page row
pub(crate) struct PageTab {
    pub page: PageRef,
    pub name: String,
}

/// an opened drawing
pub(crate) struct Opened {
    pub doc: Document,
    /// the file's name without folder and extension (the title bar)
    pub name: String,
    pub tabs: Vec<PageTab>,
    paints: Vec<Option<Rc<PagePaint>>>,
    /// the notice under the ribbon (None: closed)
    pub notice: Option<(String, String)>,
    /// counts every new document (the canvas' version)
    pub serial: u32,
    /// (JD7) undo, the preview of a gesture, the file
    pub ed: crate::edit_doc::EditState,
}

impl Opened {
    pub(crate) fn new(doc: Document, name: String, tabs: Vec<PageTab>, notice: Option<(String, String)>, serial: u32, ed: crate::edit_doc::EditState) -> Opened {
        let n = tabs.len();
        Opened { doc, name, tabs, paints: (0..n).map(|_| None).collect(), notice, serial, ed }
    }

    /// page k's paint when it is built
    pub(crate) fn paint_of(&self, k: usize) -> Option<Rc<PagePaint>> {
        self.paints.get(k).and_then(|p| p.clone())
    }

    /// (JD8 rest: pages) other tabs (pages.rs, after the pages changed): a page keeps the paint
    /// built for it
    pub(crate) fn set_tabs(&mut self, tabs: Vec<PageTab>) {
        let old: Vec<(PageRef, Option<Rc<PagePaint>>)> = self.tabs.iter().map(|t| t.page).zip(self.paints.drain(..)).collect();
        self.paints = tabs.iter().map(|t| old.iter().find(|o| o.0 == t.page).and_then(|o| o.1.clone())).collect();
        self.tabs = tabs;
    }
}

/// the extensions the file dialog offers
// i18n
pub(crate) static EXTS: [&str; 4] = ["vsdx;vsdm=Visio-Zeichnung", "vssx;vssm=Visio-Schablone", "vstx;vstm=Visio-Vorlage", "=Alle Dateien"];

/// the file types Speichern unter offers (JD7)
// i18n
pub(crate) static SAVE_EXTS: [&str; 4] = ["vsdx=Visio-Zeichnung", "vssx=Visio-Schablone", "vstx=Visio-Vorlage", "vsdm=Visio-Zeichnung mit Makros"];

/// whether a command-line argument names a drawing
pub(crate) fn is_drawing(arg: &str) -> bool {
    let a = arg.to_ascii_lowercase();
    [".vsdx", ".vsdm", ".vssx", ".vssm", ".vstx", ".vstm"].iter().any(|e| a.ends_with(e))
}

/// the installed fonts by family name (scanned on the first request)
struct Installed {
    db: Option<text::fontdb::FontDb>,
}

impl FontSource for Installed {
    fn find(&mut self, family: &str, bold: bool, italic: bool) -> Option<(Vec<u8>, u32)> {
        let db = self.db.get_or_insert_with(|| {
            let mut d = text::fontdb::FontDb::new();
            d.scan(0);
            d
        });
        let (i, syn) = db.find(family, if bold { 700 } else { 400 }, italic)?;
        // a synthetic style is the substitution's business (it leans or keeps regular)
        if syn != 0 {
            return None;
        }
        let f = db.face(i)?;
        if !f.family.eq_ignore_ascii_case(family) && !f.tfamily.eq_ignore_ascii_case(family) {
            return None;
        }
        let data = plat::read_file(&f.path)?;
        Some((data, f.face))
    }
}

/// the fonts pages are drawn with
pub(crate) fn fonts() -> FontSet {
    let mut f = FontSet::new();
    let own = ui::Style::fonts(&style::STYLE);
    // Inter Regular, Medium, SemiBold (NetLab): SemiBold stands for bold
    f.add(Class::Sans, false, false, own[0]);
    f.add(Class::Sans, true, false, own[2]);
    if !plat::args().iter().any(|a| a == "--no-system-fonts") {
        f.set_source(Box::new(Installed { db: None }));
    }
    f
}

/// read a drawing; false (and a notice) when it cannot be read
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui, path: &str) -> bool {
    let name = String::from(plat::display_name(path));
    let stem = String::from(name.rsplit_once('.').map_or(name.as_str(), |x| x.0));
    let Some(bytes) = plat::read_file(path) else {
        app.error = Some((tf("„{}“ kann nicht geöffnet werden.", &[&name]), String::from(t("Die Datei lässt sich nicht lesen."))));
        ui.repass();
        return false;
    };
    let loaded = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l,
        Err(e) => {
            app.error = Some((tf("„{}“ kann nicht geöffnet werden.", &[&name]), format!("{e}")));
            ui.repass();
            return false;
        }
    };
    let doc = loaded.doc;
    // (JD9) a stencil goes into the Shapes panel; the drawing stays
    if doc.kind == DocKind::Stencil {
        app.backstage = None;
        return crate::stencils::open_file(app, ui, path);
    }
    // the notice: macros, what the reader warned about (editing is on since JD7)
    let mut msg = String::new();
    if loaded.format.macros {
        msg.push_str(t("Die Makros der Datei bleiben erhalten und werden nie ausgeführt."));
    }
    let warnings = loaded.warnings.len() + loaded.more_warnings as usize;
    if warnings > 0 {
        if !msg.is_empty() {
            msg.push(' ');
        }
        msg.push_str(&tf("{} Hinweise beim Lesen.", &[&format!("{warnings}")]));
    }
    let notice = (!msg.is_empty()).then(|| (String::from(t("Hinweise zur Datei")), msg));
    crate::edit_doc::install(app, ui, doc, stem, Some(String::from(path)), loaded.format, notice);
    ui.recent_add(path);
    ui.repass();
    true
}

/// close the drawing: back to the empty one
pub(crate) fn close(app: &mut Diagrammer, ui: &mut Ui) {
    app.opened = None;
    app.se = SeState::new();
    app.text = None;
    app.error = None;
    app.page_tab = 0;
    app.backstage = None;
    app.canvas.request_fit();
    ui.repass();
}

/// (JD7) bring the built pages up to date with the edits since the last call: the top-level
/// shapes of the changed sheets again (diagram_view's partial update), else the page anew
pub(crate) fn sync(app: &mut Diagrammer) {
    let Some(o) = app.opened.as_mut() else { return };
    if !o.ed.rebuild && o.ed.changed.is_empty() {
        return;
    }
    let changed = core::mem::take(&mut o.ed.changed);
    let rebuild = core::mem::take(&mut o.ed.rebuild);
    for (k, slot) in o.paints.iter_mut().enumerate() {
        let Some(p) = slot.as_ref() else { continue };
        let Some(tab) = o.tabs.get(k) else { continue };
        if rebuild {
            *slot = None;
            continue;
        }
        // the top-level shapes of this page among the changed sheets
        let mut tops: Vec<diagram_core::ShapeRef> = Vec::new();
        let mut page_sheet = false;
        for s in &changed {
            match *s {
                diagram_core::SheetRef::Shape(r) if r.owner == diagram_core::Owner::Page(tab.page) => {
                    let mut top = r;
                    let mut cur = o.doc.shape(r).and_then(|x| x.parent());
                    while let Some(c) = cur {
                        top = c;
                        cur = o.doc.shape(c).and_then(|x| x.parent());
                    }
                    if !tops.contains(&top) {
                        tops.push(top);
                    }
                }
                diagram_core::SheetRef::Page(pg) if pg == tab.page => page_sheet = true,
                diagram_core::SheetRef::Shape(_) | diagram_core::SheetRef::Page(_) => {}
                // a master, a style, the document: everything may look different
                _ => page_sheet = true,
            }
        }
        if page_sheet {
            *slot = None;
            continue;
        }
        if tops.is_empty() {
            continue;
        }
        let r = p.view.try_borrow_mut().ok().and_then(|mut v| v.update(&o.doc, &mut app.fonts, &tops));
        match r {
            // the changed part in world units for ui.diagram's backing store (DgPaint::damage)
            Some(b) => p.note_damage(canvas_ver(o.serial, k, o.ed.rev), [(b[0] * DIP) as f32, ((p.h - b[3]) * DIP) as f32, (b[2] * DIP) as f32, ((p.h - b[1]) * DIP) as f32]),
            None => *slot = None,
        }
    }
}

/// (JD7) the canvas version of page k: changes with the drawing and every edit of it
pub(crate) fn canvas_ver(serial: u32, k: usize, rev: u32) -> u32 {
    serial.wrapping_mul(4096).wrapping_add(k as u32 + 1).wrapping_add(rev.wrapping_mul(0x9E37_79B9))
}

/// (JD7) the text measures of the formula engine: the fonts pages are drawn with (TEXTWIDTH and
/// TEXTHEIGHT give what is drawn)
pub(crate) fn metrics(app: &Diagrammer) -> Rc<dyn diagram_core::formula::TextMetrics> {
    Rc::new(diagram_view::Metrics::new(&app.fonts))
}

/// page k of the open drawing, built on first use
pub(crate) fn paint(app: &mut Diagrammer, k: usize) -> Option<Rc<PagePaint>> {
    sync(app);
    let o = app.opened.as_mut()?;
    let tab = o.tabs.get(k)?;
    if let Some(Some(p)) = o.paints.get(k) {
        return Some(p.clone());
    }
    let view = PageView::new(&o.doc, tab.page, &mut app.fonts, &Options::default());
    let d = view.drawing();
    let (w, h, color) = (d.width, d.height, d.color);
    // fonts drawn with a substitute: named in the notice (TX-09)
    let subs = view.substitutions().join(", ");
    if !subs.is_empty() {
        if let Some((_, msg)) = o.notice.as_mut() {
            let line = tf("Ersetzte Schriften: {}.", &[&subs]);
            if !msg.contains(&line) {
                msg.push(' ');
                msg.push_str(&line);
            }
        }
    }
    let p = Rc::new(PagePaint { view: RefCell::new(view), w, h, color, damage: RefCell::new(Vec::new()) });
    o.paints[k] = Some(p.clone());
    Some(p)
}

/// the canvas' model: the page's sheet as one node (fit and overview go by it)
pub(crate) fn sheet_node(app: &Diagrammer, k: usize) -> Option<DNode> {
    let o = app.opened.as_ref()?;
    let p = o.paints.get(k)?.as_ref()?;
    Some(DNode { key: 1, rect: [0.0, 0.0, (p.w * DIP) as f32, (p.h * DIP) as f32], flags: DN_LOCKED | DN_NOSELECT | DN_OUTLINE, ..DNode::default() })
}

/// zoom so that the page's width fills the canvas (Ansicht › Seitenbreite)
pub(crate) fn page_width(app: &mut Diagrammer) {
    let k = app.page_tab.max(0) as usize;
    let Some(n) = sheet_node(app, k) else {
        app.canvas.request_fit();
        return;
    };
    let vp = &mut app.canvas.vp;
    let d = vp.dpi.max(1) as f32 / 96.0;
    let avail = (vp.rect[2] as f32 - 48.0).max(1.0);
    let z = (avail / (n.rect[2].max(1.0) * d)).clamp(Viewport::ZOOM_MIN, Viewport::ZOOM_MAX);
    let r = vp.rect;
    vp.zoom_at(z, r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
    vp.cx = (n.rect[2] * 0.5) as f64;
}

/// to the nearest whole pixel
fn ri(v: f32) -> i32 {
    if v >= 0.0 { (v + 0.5) as i32 } else { (v - 0.5) as i32 }
}

/// the first argument that names a drawing (the values of options skipped)
pub(crate) fn command_line_file() -> Option<String> {
    let args = plat::args();
    let mut skip = false;
    for a in args.iter().skip(1) {
        if skip {
            skip = false;
            continue;
        }
        if plat::VALUE_OPTS.contains(&a.as_str()) || a == "--shell-preview" || a == "--config" || a == "--save-dir" {
            skip = true;
            continue;
        }
        if !a.starts_with('-') && is_drawing(a) {
            return Some(a.clone());
        }
    }
    None
}

/// a page drawn into the canvas
pub(crate) struct PagePaint {
    view: RefCell<PageView>,
    /// the page in paper inches, its colour
    w: f64,
    h: f64,
    color: u32,
    /// (JD7) what each edit changed: the canvas version after it, the world box
    damage: RefCell<Vec<(u32, [f32; 4])>>,
}

impl PagePaint {
    /// note the world box an edit changed, reaching version `ver`
    fn note_damage(&self, ver: u32, b: [f32; 4]) {
        let mut d = self.damage.borrow_mut();
        if d.len() >= 64 {
            d.remove(0);
        }
        d.push((ver, b));
    }

    /// the page view (its display list for hit tests)
    pub(crate) fn view(&self) -> core::cell::Ref<'_, PageView> {
        self.view.borrow()
    }

    /// (JD8 rest) the page's size in paper inches
    pub(crate) fn size(&self) -> (f64, f64) {
        (self.w, self.h)
    }

    /// (JD8 rest) the page fitted into w x h pixels (Verschieben und Zoomen)
    pub(crate) fn thumbnail(&self, w: u32, h: u32) -> Option<gfx::Surface> {
        self.view.try_borrow_mut().ok().map(|mut v| v.render(w.max(8), h.max(8)))
    }

    /// render2d's camera for the canvas' view: the paper point at the middle, inches per pixel
    fn camera(&self, vp: &Viewport) -> Camera {
        let s = vp.scale();
        Camera::new([vp.cx / DIP, self.h - vp.cy / DIP], 1.0 / (DIP * s))
    }
}

impl DgPaint for PagePaint {
    /// the page's sheet: a shadow, the paper
    fn paint_under(&self, g: &mut Gfx, f: &DgFrame) {
        let r = f.vp.rect_to_screen([0.0, 0.0, (self.w * DIP) as f32, (self.h * DIP) as f32]);
        let (x, y) = (ri(r[0]), ri(r[1]));
        let (w, h) = (ri(r[0] + r[2]) - x, ri(r[1] + r[3]) - y);
        let sh = ri(f.vp.dpi as f32 / 96.0 * 3.0).max(1);
        g.dc.set_color(if f.dark { 0x6000_0000 } else { 0x3000_0000 });
        g.dc.fill(g.sf, x + sh, y + sh, w, h);
        g.dc.set_color(0xFF00_0000 | self.color);
        g.dc.fill(g.sf, x, y, w, h);
    }

    /// (JD7) the box the edits from version `from` to `to` changed (None: not known, all of it)
    fn damage(&self, from: u32, to: u32) -> Option<[f32; 4]> {
        let d = self.damage.borrow();
        let start = d.iter().position(|e| e.0 == from)? + 1;
        let end = d.iter().rposition(|e| e.0 == to)?;
        if end < start {
            return None;
        }
        d[start..=end].iter().map(|e| e.1).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
    }

    /// the page's shapes
    fn paint(&self, g: &mut Gfx, f: &DgFrame) {
        let Ok(mut v) = self.view.try_borrow_mut() else { return };
        let cam = self.camera(&f.vp);
        v.draw(g.dc, g.sf, f.vp.rect, &cam);
    }
}
