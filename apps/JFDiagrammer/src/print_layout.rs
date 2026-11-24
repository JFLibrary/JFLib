// print_layout -- printing (JD12: PR-01 .. PR-05, EX-15): the print setup, sheets and tiles, the
// pictures of the preview, the job through the system's printing (plat::PrintDoc)
//
//   PrintSetup::from_page(doc, page)   paper, orientation, zoom or "fit to n x m sheets",
//                                      margins, centring, as the page's Print Properties say
//                                      (PaperKind, PrintPageOrientation, ScaleX, PagesX / PagesY,
//                                      Page*Margin, CenterX / CenterY); Visio's defaults else
//   tiling(page, setup, paper, edges) -> Tiling   the sheets a page needs and where it lies on
//                                      each (PR-03): a page that fits on the paper takes one
//                                      sheet, a larger one is cut into tiles of the printable
//                                      area, from the top left, row by row; never cut off
//   render_sheet(view, tiling, k, dpi, crop, gray, grid) -> Surface   a sheet as pixels (preview,
//                                      Windows' printers); (JD15, PR-05) with the grid's lines
//                                      under the page when the setup prints the grid
//                                      (grid_step: the canvas's step at 100 %; the PDF way
//                                      draws them as hairlines under the items)
//   print(app, job) -> Result<sheets, message>   the job: the system's print dialog (printer,
//                                      paper, copies, range), then vector PDF pages where the
//                                      platform prints PDF (Linux: the portal), else every sheet
//                                      as a 300-DPI picture (Windows: GDI)
//
// Coordinates: paper and page in inches, y up from the bottom left corner (the display list's
// way); a sheet's `place` maps page inches to paper inches (scale and move, no turn), `clip` is
// the part of the paper the page may print on (the printable area when tiled, the whole paper
// when it fits).  The test variant of plat (jfgui_PRINT_TO) prints into a file, never on paper.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::geometry::Xform;
use diagram_core::render::{self, Drawing, ItemKind, Paint};
use diagram_core::{CellAddr, Document, PageRef, SheetRef};
use diagram_view::export::pdf::Sheets;
use diagram_view::PageView;
use gfx::{Dc, Surface, FMT_XRGB};
use render2d::Camera;
use ui::i18n::{t, tf};

use crate::Diagrammer;

/// the papers on offer: name, plat's paper, width and height in inches (portrait)
pub(crate) const PAPERS: [(&str, u32, f64, f64); 5] = [
    ("A4", plat::PAPER_A4, 210.0 / 25.4, 297.0 / 25.4),
    ("A3", plat::PAPER_A3, 297.0 / 25.4, 420.0 / 25.4),
    ("A5", plat::PAPER_A5, 148.0 / 25.4, 210.0 / 25.4),
    ("Letter", plat::PAPER_LETTER, 8.5, 11.0),
    ("Legal", plat::PAPER_LEGAL, 8.5, 14.0),
];

/// the resolution sheets are printed with where the platform takes pictures only
pub(crate) const PRINT_DPI: f64 = 300.0;

/// how large the page prints
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fit {
    /// a zoom (1.0 = 100 %)
    Percent(f64),
    /// as large as fits on n sheets across, m down
    Sheets(u32, u32),
}

/// how pages print (Datei › Drucken, Seite einrichten › Drucken einrichten)
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PrintSetup {
    /// an index into PAPERS
    pub paper: usize,
    pub landscape: bool,
    pub fit: Fit,
    /// left, top, right, bottom (inches)
    pub margins: [f64; 4],
    /// centre on the sheet across, down (when the page fits on one)
    pub center: [bool; 2],
    /// background pages print under the pages
    pub background: bool,
    /// grey instead of colours
    pub gray: bool,
    /// (JD15, PR-05) the grid prints under the page (Gitternetz drucken, Visio's PrintGrid)
    pub grid: bool,
}

impl Default for PrintSetup {
    fn default() -> PrintSetup {
        PrintSetup { paper: 0, landscape: false, fit: Fit::Percent(1.0), margins: [0.25; 4], center: [false; 2], background: true, gray: false, grid: false }
    }
}

impl PrintSetup {
    /// the setup the page's Print Properties name (Visio's defaults for what is missing)
    pub(crate) fn from_page(doc: &Document, page: PageRef) -> PrintSetup {
        let mut s = PrintSetup::default();
        let num = |name: &str| doc.sym(name).and_then(|c| doc.resolve(SheetRef::Page(page), &CellAddr::single(c)).num()).filter(|v| v.is_finite());
        // DMPAPER codes: 1 Letter, 5 Legal, 8 A3, 9 A4, 11 A5
        s.paper = match num("PaperKind").map(|v| v as i64) {
            Some(1) => 3,
            Some(5) => 4,
            Some(8) => 1,
            Some(11) => 2,
            _ => 0,
        };
        s.landscape = num("PrintPageOrientation") == Some(2.0);
        if let (Some(x), Some(y)) = (num("PagesX"), num("PagesY")) {
            if x >= 1.0 && y >= 1.0 && (x > 1.0 || y > 1.0) {
                s.fit = Fit::Sheets(x.min(99.0) as u32, y.min(99.0) as u32);
            }
        }
        if s.fit == Fit::Percent(1.0) {
            if let Some(z) = num("ScaleX").filter(|z| *z > 0.01 && *z < 100.0) {
                s.fit = Fit::Percent(z);
            }
        }
        for (k, name) in ["PageLeftMargin", "PageTopMargin", "PageRightMargin", "PageBottomMargin"].iter().enumerate() {
            if let Some(m) = num(name).filter(|m| *m >= 0.0 && *m < 5.0) {
                s.margins[k] = m;
            }
        }
        s.center = [num("CenterX").is_some_and(|v| v != 0.0), num("CenterY").is_some_and(|v| v != 0.0)];
        s.grid = num("PrintGrid").is_some_and(|v| v != 0.0);
        s
    }

    /// the paper in inches, the orientation applied
    pub(crate) fn paper_size(&self) -> (f64, f64) {
        let p = PAPERS[self.paper.min(PAPERS.len() - 1)];
        if self.landscape { (p.3, p.2) } else { (p.2, p.3) }
    }
}

/// a sheet of a page
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sheet {
    /// page inches -> paper inches (y up)
    pub place: Xform,
    /// where the page may print on the paper (x0, y0, x1, y1; inches, y up)
    pub clip: [f64; 4],
    pub col: u32,
    pub row: u32,
}

/// the sheets of a page
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tiling {
    /// the paper (inches)
    pub paper: (f64, f64),
    /// paper inches per page inch
    pub scale: f64,
    pub cols: u32,
    pub rows: u32,
    pub sheets: Vec<Sheet>,
}

/// at most this many sheets per page (a zoom of 1000 % on a large page stops here)
const MAX_SHEETS: u32 = 400;

/// the sheets a page of `page` inches needs on `paper` (inches) with the printer's unprintable
/// edges `edges` (left, top, right, bottom; inches)
pub(crate) fn tiling(page: (f64, f64), setup: &PrintSetup, paper: (f64, f64), edges: [f64; 4]) -> Tiling {
    let (pw, ph) = (paper.0.max(0.5), paper.1.max(0.5));
    let (w, h) = (page.0.max(1e-6), page.1.max(1e-6));
    let m: Vec<f64> = (0..4).map(|k| setup.margins[k].max(edges[k]).max(0.0)).collect();
    let aw = (pw - m[0] - m[2]).max(0.1);
    let ah = (ph - m[1] - m[3]).max(0.1);
    let mut s = match setup.fit {
        Fit::Percent(p) if p.is_finite() && p > 0.0 => p,
        Fit::Sheets(n, k) => (n.max(1) as f64 * aw / w).min(k.max(1) as f64 * ah / h),
        _ => 1.0,
    };
    let eps = 1e-6;
    let (sw, sh) = (w * s, h * s);
    let area = [m[0], m[3], pw - m[2], ph - m[1]];
    let place = |x0: f64, top: f64, s: f64| Xform { a: s, b: 0.0, c: 0.0, d: s, e: x0, f: ph - top - s * h };
    if sw <= aw + eps && sh <= ah + eps {
        let x0 = m[0] + if setup.center[0] { (aw - sw) * 0.5 } else { 0.0 };
        let top = m[1] + if setup.center[1] { (ah - sh) * 0.5 } else { 0.0 };
        return Tiling { paper: (pw, ph), scale: s, cols: 1, rows: 1, sheets: alloc::vec![Sheet { place: place(x0, top, s), clip: area, col: 0, row: 0 }] };
    }
    if sw <= pw + eps && sh <= ph + eps {
        // as large as the paper: on it as it is (what lies in the printer's edge is the printer's)
        let (x0, top) = ((pw - sw) * 0.5, (ph - sh) * 0.5);
        return Tiling { paper: (pw, ph), scale: s, cols: 1, rows: 1, sheets: alloc::vec![Sheet { place: place(x0, top, s), clip: [0.0, 0.0, pw, ph], col: 0, row: 0 }] };
    }
    let mut cols = ceil(sw / aw - 1e-9).max(1.0) as u32;
    let mut rows = ceil(sh / ah - 1e-9).max(1.0) as u32;
    while cols as u64 * rows as u64 > MAX_SHEETS as u64 {
        // too many: smaller until the page fits on MAX_SHEETS
        s *= 0.9;
        cols = ceil(w * s / aw - 1e-9).max(1.0) as u32;
        rows = ceil(h * s / ah - 1e-9).max(1.0) as u32;
    }
    let mut sheets = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            // tile (c, r) shows the scaled page from c * aw (left) and r * ah (top)
            let x0 = m[0] - c as f64 * aw;
            let top = m[1] - r as f64 * ah;
            sheets.push(Sheet { place: place(x0, top, s), clip: area, col: c, row: r });
        }
    }
    Tiling { paper: (pw, ph), scale: s, cols, rows, sheets }
}

fn ceil(v: f64) -> f64 {
    let t = v as i64 as f64;
    if t < v { t + 1.0 } else { t }
}

/// a sheet as pixels: the paper part `crop` (inches, y up; None: the whole sheet) at `dpi`,
/// white paper, the page on it through the view's CPU path, grey when asked; (JD15, PR-05)
/// `grid` > 0: the grid's lines every `grid` page inches under the page
pub(crate) fn render_sheet(view: &mut PageView, t: &Tiling, k: usize, dpi: f64, crop: Option<[f64; 4]>, gray: bool, grid: f64) -> Option<Surface> {
    let sheet = t.sheets.get(k)?;
    let r = crop.unwrap_or([0.0, 0.0, t.paper.0, t.paper.1]);
    let px = |v: f64| (v * dpi + 0.5) as i64;
    let (w, h) = (px(r[2] - r[0]).max(1), px(r[3] - r[1]).max(1));
    if w > 30_000 || h > 30_000 {
        return None;
    }
    let mut sf = Surface::try_new(w as i32, h as i32, FMT_XRGB)?;
    let mut dc = Dc::new(w as i32, h as i32);
    dc.begin(&sf);
    dc.clear(&mut sf, 0xFFFF_FFFF);
    // the clip in the picture's pixels
    let c = [sheet.clip[0].max(r[0]), sheet.clip[1].max(r[1]), sheet.clip[2].min(r[2]), sheet.clip[3].min(r[3])];
    let (x0, y0) = (px(c[0] - r[0]), px(r[3] - c[3]));
    let (x1, y1) = (px(c[2] - r[0]), px(r[3] - c[1]));
    if x1 > x0 && y1 > y0 {
        // the paper point at the middle of that rectangle, back on the page
        let mid = [r[0] + (x0 + x1) as f64 / (2.0 * dpi), r[3] - (y0 + y1) as f64 / (2.0 * dpi)];
        let p = &sheet.place;
        let page = [(mid[0] - p.e) / p.a, (mid[1] - p.f) / p.d];
        let cam = Camera::new(page, 1.0 / (dpi * p.a));
        if grid > 0.0 {
            dc.clip_push(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32);
            dc.set_color(GRID_COLOR);
            let (pw, ph) = (view.drawing().width, view.drawing().height);
            for (x, y) in grid_lines(pw, ph, grid) {
                // page inches -> paper inches -> pixels of the picture
                match (x, y) {
                    (Some(x), _) => dc.vline(&mut sf, px(p.a * x + p.e - r[0]) as i32, px(r[3] - (p.d * ph + p.f)) as i32, px(p.d * ph) as i32 + 1),
                    (_, Some(y)) => dc.hline(&mut sf, px(p.e - r[0]) as i32, px(r[3] - (p.d * y + p.f)) as i32, px(p.a * pw) as i32 + 1),
                    _ => {}
                }
            }
            dc.clip_pop();
        }
        view.draw(&mut dc, &mut sf, [x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32], &cam);
    }
    dc.end();
    if gray {
        to_gray(&mut sf);
    }
    Some(sf)
}

// (JD15, PR-05) the printed grid: a light grey, as Visio prints its grid
const GRID_COLOR: u32 = 0xFFC8_CCD2;

/// (JD15, PR-05) the grid's step on the page (page inches) as the canvas shows it at 100 %:
/// the first of the canvas's steps (mm or inches of the drawing, edit_canvas::grid_step) that is
/// at least 1/6 in on the page
pub(crate) fn grid_step(doc: &Document, page: PageRef) -> f64 {
    let s = SheetRef::Page(page);
    let num = |name: &str| doc.sym(name).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num()).filter(|v| v.is_finite() && *v > 0.0);
    let scale = num("PageScale").unwrap_or(1.0) / num("DrawingScale").unwrap_or(1.0);
    let metric = doc.sym("PageWidth").map(|c| doc.resolve(s, &CellAddr::single(c)).unit()).map_or(true, |u| {
        use diagram_core::Unit::*;
        matches!(u, Some(Millimeters) | Some(Centimeters) | Some(Meters) | Some(Kilometers))
    });
    let unit = if metric { scale / 25.4 } else { scale };
    let steps: &[f64] = if metric { &[0.5, 1.0, 2.0, 2.5, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0] } else { &[1.0 / 32.0, 1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 6.0, 12.0, 24.0, 60.0, 120.0] };
    steps.iter().map(|s| s * unit).find(|&s| s >= 1.0 / 6.0).unwrap_or(steps[steps.len() - 1] * unit)
}

/// the grid's lines of a page of w x h page inches every `step`: (Some(x), None) a vertical
/// line, (None, Some(y)) a horizontal one (at most 2000 of each)
fn grid_lines(w: f64, h: f64, step: f64) -> Vec<(Option<f64>, Option<f64>)> {
    let mut out = Vec::new();
    if !(step > 1e-6) {
        return out;
    }
    let (nx, ny) = (((w / step) as usize).min(2000), ((h / step) as usize).min(2000));
    out.extend((0..=nx).map(|k| (Some(k as f64 * step), None)));
    out.extend((0..=ny).map(|k| (None, Some(k as f64 * step))));
    out
}

/// (JD15, PR-05) the drawing with the grid's lines under its items (the PDF way)
fn with_grid(d: &Drawing, step: f64) -> Drawing {
    use diagram_core::geometry::{Path, Seg, Sub};
    let Some(first) = d.items.first() else { return d.clone() };
    let mut path = Path::default();
    for l in grid_lines(d.width, d.height, step) {
        let (a, b) = match l {
            (Some(x), _) => ([x, 0.0], [x, d.height]),
            (_, Some(y)) => ([0.0, y], [d.width, y]),
            _ => continue,
        };
        path.subs.push(Sub { start: a, segs: alloc::vec![Seg::Line(b)], closed: false });
    }
    let stroke = render::Stroke { color: GRID_COLOR, width: 0.0, dash: Vec::new(), cap: render::Cap::Butt, join: render::Join::Miter };
    let item = render::Item { shape: first.shape, top: first.top, bbox: [0.0, 0.0, d.width, d.height], role: render::Role::Shape, kind: ItemKind::Stroke { path, stroke } };
    let mut out = d.clone();
    out.items.insert(0, item);
    out
}

/// the pixels as grey (luma, Rec. 601)
pub(crate) fn to_gray(sf: &mut Surface) {
    let stride = sf.stride as usize;
    for y in 0..sf.h.max(0) as usize {
        for x in 0..sf.w.max(0) as usize {
            let i = y * stride + x * 4;
            let (b, g, r) = (sf.px[i] as u32, sf.px[i + 1] as u32, sf.px[i + 2] as u32);
            let l = ((r * 299 + g * 587 + b * 114 + 500) / 1000) as u8;
            sf.px[i] = l;
            sf.px[i + 1] = l;
            sf.px[i + 2] = l;
        }
    }
}

/// the display list in grey (the PDF way of PR-01's Graustufen; pictures keep their colours)
pub(crate) fn gray_drawing(d: &Drawing) -> Drawing {
    fn g(c: u32) -> u32 {
        let (r, gg, b) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
        let l = (r * 299 + gg * 587 + b * 114 + 500) / 1000;
        (c & 0xFF00_0000) | l << 16 | l << 8 | l
    }
    let mut out = d.clone();
    out.color = g(out.color);
    for it in out.items.iter_mut() {
        match &mut it.kind {
            ItemKind::Fill { paint, .. } => match paint {
                Paint::Solid(c) => *c = g(*c),
                Paint::Hatch { color, .. } => *color = g(*color),
                Paint::Gradient(gr) => {
                    for s in gr.stops.iter_mut() {
                        s.1 = g(s.1);
                    }
                }
            },
            ItemKind::Stroke { stroke, .. } => stroke.color = g(stroke.color),
            ItemKind::Text(tx) => {
                for r in tx.runs.iter_mut() {
                    r.color = g(r.color);
                }
            }
            ItemKind::Image(_) => {}
        }
    }
    out
}

/// what to print
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Job {
    pub pages: Vec<PageRef>,
    pub copies: u32,
    pub setup: PrintSetup,
    pub title: String,
}

/// print the job: the system's dialog (where there is one), then the sheets; Ok(sheets
/// printed) or the message to show
pub(crate) fn print(app: &mut Diagrammer, job: &Job) -> Result<u32, String> {
    let Some(o) = app.opened.as_ref() else { return Err(String::from(t("Es ist keine Zeichnung geöffnet."))) };
    let doc = &o.doc;
    if job.pages.is_empty() {
        return Err(String::from(t("Es sind keine Seiten zum Drucken gewählt.")));
    }
    let opts = render::Options { background: job.setup.background, hidden_layers: false, print: true };
    // the views (texts measured with the fonts drawn) and the sheets on the chosen paper
    let mut views: Vec<PageView> = job.pages.iter().map(|p| PageView::new(doc, *p, &mut app.fonts, &opts)).collect();
    let count = |paper: (f64, f64), edges: [f64; 4], views: &[PageView]| -> u32 {
        views.iter().map(|v| tiling((v.drawing().width, v.drawing().height), &job.setup, paper, edges).sheets.len() as u32).sum::<u32>() * job.copies.max(1)
    };
    let total = count(job.setup.paper_size(), [0.0; 4], &views);
    let mut pj = plat::PrintJob::new(None, &job.title);
    pj.paper = PAPERS[job.setup.paper.min(PAPERS.len() - 1)].1;
    pj.flags = if job.setup.landscape { plat::PJF_LANDSCAPE } else { 0 };
    pj.scale = plat::PJS_DPI;
    pj.dpi_x = PRINT_DPI;
    pj.dpi_y = PRINT_DPI;
    pj.margin = 0;
    let mut d = match plat::PrintDoc::begin(&pj, total) {
        Ok(d) => d,
        Err(plat::ERR_CANCEL) => return Err(String::from(t("Drucken abgebrochen."))),
        Err(plat::ERR_UNSUPPORTED) => return Err(String::from(t("Drucken ist auf diesem System oder ohne Berechtigung nicht möglich."))),
        Err(e) => return Err(tf("Drucken fehlgeschlagen (Fehler {}).", &[&alloc::format!("{e}")])),
    };
    // the paper the dialog chose, the printer's unprintable edges
    let pp = d.page();
    let paper = (pp.w / 72.0, pp.h / 72.0);
    let edges = [pp.l / 72.0, pp.t / 72.0, pp.r / 72.0, pp.b / 72.0];
    let tilings: Vec<Tiling> = views.iter().map(|v| tiling((v.drawing().width, v.drawing().height), &job.setup, paper, edges)).collect();
    // (JD15, PR-05) the grid's step of each page (0: no grid)
    let grids: Vec<f64> = job.pages.iter().map(|p| if job.setup.grid { grid_step(doc, *p) } else { 0.0 }).collect();
    let mut n = 0u32;
    let mut printed = 0u32;
    if d.pdf_native() {
        // vector pages (Linux: the portal prints our PDF as it is)
        let pdf = {
            let mut sheets = Sheets::new(&mut app.fonts);
            for _ in 0..job.copies.max(1) {
                for ((v, tl), &grid) in views.iter().zip(&tilings).zip(&grids) {
                    let drawing = if job.setup.gray { gray_drawing(v.drawing()) } else { v.drawing().clone() };
                    let drawing = if grid > 0.0 { with_grid(&drawing, grid) } else { drawing };
                    for sh in &tl.sheets {
                        n += 1;
                        if !d.wants(n) {
                            continue;
                        }
                        let place = sh.place.then(&Xform { a: 72.0, b: 0.0, c: 0.0, d: 72.0, e: 0.0, f: 0.0 });
                        let clip = [sh.clip[0] * 72.0, sh.clip[1] * 72.0, sh.clip[2] * 72.0, sh.clip[3] * 72.0];
                        sheets.add(doc, &drawing, (pp.w, pp.h), place, Some(clip), &[], false);
                        printed += 1;
                    }
                }
            }
            sheets.pdf().set_info(pdf::Info { title: Some(job.title.clone()), creator: Some(String::from("JFDiagrammer")), ..Default::default() });
            sheets.finish()
        };
        match pdf {
            Ok(bytes) if printed > 0 => {
                if d.add_pdf(&bytes) != 0 {
                    d.abort();
                    return Err(String::from(t("Der Drucker hat das Dokument nicht angenommen.")));
                }
            }
            Ok(_) => {
                d.abort();
                return Err(String::from(t("Im gewählten Bereich liegt kein Blatt.")));
            }
            Err(e) => {
                d.abort();
                return Err(tf("Drucken fehlgeschlagen: {}", &[&alloc::format!("{e}")]));
            }
        }
    } else {
        // pictures (Windows: GDI): the printable area of each sheet at PRINT_DPI
        let crop = [edges[0], edges[3], paper.0 - edges[2], paper.1 - edges[1]];
        for _ in 0..job.copies.max(1) {
            for ((v, tl), &grid) in views.iter_mut().zip(&tilings).zip(&grids) {
                for k in 0..tl.sheets.len() {
                    n += 1;
                    if !d.wants(n) {
                        continue;
                    }
                    let Some(sf) = render_sheet(v, tl, k, PRINT_DPI, Some(crop), job.setup.gray, grid) else {
                        d.abort();
                        return Err(String::from(t("Für das Blatt reicht der Speicher nicht.")));
                    };
                    let src = plat::PrintSrc { px: &sf.px, w: sf.w, h: sf.h, stride: sf.stride, fmt: plat::PRINT_XRGB };
                    let r = d.add_image(&src);
                    if r != 0 {
                        d.abort();
                        return Err(tf("Drucken fehlgeschlagen (Fehler {}).", &[&alloc::format!("{r}")]));
                    }
                    printed += 1;
                }
            }
        }
        if printed == 0 {
            d.abort();
            return Err(String::from(t("Im gewählten Bereich liegt kein Blatt.")));
        }
    }
    match d.end() {
        0 => Ok(printed),
        plat::ERR_CANCEL => Err(String::from(t("Drucken abgebrochen."))),
        e => Err(tf("Drucken fehlgeschlagen (Fehler {}).", &[&alloc::format!("{e}")])),
    }
}
