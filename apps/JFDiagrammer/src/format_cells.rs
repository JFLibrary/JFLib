// format_cells -- (JD11) the format of the selection: what its first shape shows, and changes
// to every selected shape as one undo step (Start › Schriftart, Absatz, Formenarten; Shape
// formatieren; Format übertragen)
//
//   F::Cell(name) / F::Char(name) / F::Para(name)   a cell of the shape, of every Character
//                                   row, of every Paragraph row
//   value(app, shape, f)            its value as the engine computes it (a "Themed" cell
//                                   worked out); num / color / text for the first selected shape
//   set(app, ui, label, &[(f, formula)])   the formulas into every selected shape (guarded
//                                   cells are left as they are)
//   inherit(app, ui, label, &[f])   the shapes' own cells go: they inherit again (the design)
//   palette(app) -> Palette         the colour menu of the page's design (FM-08): white,
//                                   black, light, dark, accents 1-6 with shades, the variant's
//                                   colours, JFLib's fixed colours, the last used
//   color_formula(pick)             what a pick writes: a design colour follows the design
//                                   (THEMEVAL with MSOTINT for a shade), a fixed one does not
//                                   (RGB); both inside THEMEGUARD, so that a design applied
//                                   later keeps the user's choice (as Visio writes them)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::design::apply;
use diagram_core::formula::{Color, Input, Outcome, Val};
use diagram_core::render::ThemeInfo;
use diagram_core::{CellAddr, Document, EditError, RowKey, SecKey, ShapeRef, SheetRef};
use ui::i18n::t;
use ui::*;

use crate::Diagrammer;

/// a format cell
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum F {
    Cell(&'static str),
    Char(&'static str),
    Para(&'static str),
}

/// the shapes selected
pub(crate) fn selection(app: &Diagrammer) -> Vec<ShapeRef> {
    if app.opened.is_none() {
        return Vec::new();
    }
    crate::edit_canvas::selected(app)
}

// the addresses of a format cell in a sheet (Character / Paragraph: every row there is)
fn addrs(doc: &Document, sheet: SheetRef, f: F) -> Vec<CellAddr> {
    let (sec, name) = match f {
        F::Cell(n) => return doc.sym(n).map(|c| alloc::vec![CellAddr::single(c)]).unwrap_or_default(),
        F::Char(n) => ("Character", n),
        F::Para(n) => ("Paragraph", n),
    };
    let (Some(s), Some(c)) = (doc.sym(sec), doc.sym(name)) else { return Vec::new() };
    let sk = SecKey::one(s);
    let mut keys: Vec<RowKey> = doc.rows(sheet, sk).into_iter().map(|r| r.key).collect();
    if keys.is_empty() {
        keys.push(RowKey::Index(0));
    }
    keys.into_iter().map(|k| CellAddr::row(sk, k, c)).collect()
}

fn plain(v: Val) -> Val {
    match v {
        Val::Themed(inner) => plain((*inner).clone()),
        v => v,
    }
}

/// the value of a format cell of a shape (the first row's for Character / Paragraph)
pub(crate) fn value(app: &mut Diagrammer, s: ShapeRef, f: F) -> Option<Val> {
    let o = app.opened.as_mut()?;
    let sheet = SheetRef::Shape(s);
    let a = *addrs(&o.doc, sheet, f).first()?;
    match o.ed.eng.evaluate(&o.doc, sheet, &a) {
        Outcome::Value(v) => Some(plain(v)),
        Outcome::Kept(_) => {
            let v = o.doc.resolve(sheet, &a).value();
            match v {
                diagram_core::Value::Num(x) => Some(Val::Num(x)),
                diagram_core::Value::Text(t) => Some(Val::Str(alloc::rc::Rc::from(o.doc.str(t)))),
                diagram_core::Value::Absent => None,
            }
        }
    }
}

/// a number of the first selected shape
pub(crate) fn num(app: &mut Diagrammer, f: F) -> Option<f64> {
    let s = *selection(app).first()?;
    match value(app, s, f)? {
        Val::Num(x) => Some(x),
        Val::Color(Color::Index(i)) => Some(i as f64),
        _ => None,
    }
}

/// a colour of the first selected shape (0xRRGGBB)
pub(crate) fn color(app: &mut Diagrammer, f: F) -> Option<u32> {
    let s = *selection(app).first()?;
    let doc_colors = |app: &Diagrammer, i: u32| -> u32 {
        let doc = &app.opened.as_ref().map(|o| &o.doc);
        doc.and_then(|d| d.colors.iter().find(|c| c.ix == i).and_then(|c| diagram_core::render::parse_color(d.str(c.rgb))))
            .unwrap_or_else(|| diagram_core::render::style::PALETTE.get(i as usize).copied().unwrap_or(0))
    };
    match value(app, s, f)? {
        Val::Color(Color::Rgb(r, g, b)) => Some(((r as u32) << 16) | ((g as u32) << 8) | b as u32),
        Val::Color(Color::Index(i)) => Some(doc_colors(app, i)),
        Val::Num(x) if x >= 0.0 => Some(doc_colors(app, x as u32)),
        Val::Str(s) => diagram_core::render::parse_color(&s),
        _ => None,
    }
}

/// a text (a font's name) of the first selected shape
pub(crate) fn text(app: &mut Diagrammer, f: F) -> Option<String> {
    let s = *selection(app).first()?;
    match value(app, s, f)? {
        Val::Str(s) | Val::Font(s) => Some(String::from(&*s)),
        _ => None,
    }
}

/// write formulas into the format cells of every selected shape, one undo step
pub(crate) fn set(app: &mut Diagrammer, ui: &mut Ui, label: &str, changes: &[(F, String)]) -> bool {
    let shapes = selection(app);
    set_on(app, ui, label, &shapes, changes)
}

/// the same for these shapes
pub(crate) fn set_on(app: &mut Diagrammer, ui: &mut Ui, label: &str, shapes: &[ShapeRef], changes: &[(F, String)]) -> bool {
    if shapes.is_empty() {
        return false;
    }
    let Some(o) = app.opened.as_mut() else { return false };
    let r = crate::edit_doc::edit(o, label, true, |tx, eng| {
        for &s in shapes {
            let sheet = SheetRef::Shape(s);
            for (f, formula) in changes {
                for a in addrs(tx.doc(), sheet, *f) {
                    match eng.user_set(tx, sheet, &a, Input::Formula(formula)) {
                        Ok(_) | Err(EditError::Guarded) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(())
    });
    ui.repass();
    r.is_ok()
}

/// a restyle (a quick style): the shapes' own cells of `clear` go (also THEMEGUARD formats;
/// GUARD stays), then the formulas of `changes` are written -- one undo step
pub(crate) fn restyle(app: &mut Diagrammer, ui: &mut Ui, label: &str, clear: &[F], changes: &[(F, String)]) -> bool {
    let shapes = selection(app);
    let Some(o) = app.opened.as_mut() else { return false };
    if shapes.is_empty() {
        return false;
    }
    let r = crate::edit_doc::edit(o, label, true, |tx, eng| {
        for &s in &shapes {
            let sheet = SheetRef::Shape(s);
            for f in clear {
                for a in addrs(tx.doc(), sheet, *f) {
                    let own = tx.doc().sheet(sheet).is_some_and(|sh| match (a.section, a.row) {
                        (None, _) => sh.cells.iter().any(|c| c.name == a.cell && c.formula != diagram_core::Formula::Inherited),
                        _ => true,
                    });
                    if own && !eng.guarded(tx.doc(), sheet, &a) {
                        tx.inherit_cell(sheet, &a)?;
                    }
                }
            }
            for (f, formula) in changes {
                for a in addrs(tx.doc(), sheet, *f) {
                    match eng.user_set(tx, sheet, &a, Input::Formula(formula)) {
                        Ok(_) | Err(EditError::Guarded) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        let sheets: Vec<SheetRef> = shapes.iter().map(|s| SheetRef::Shape(*s)).collect();
        eng.recalc(tx, &sheets)?;
        Ok(())
    });
    ui.repass();
    r.is_ok()
}

/// the shapes' own cells go: they inherit again (from the master, the style, the design)
pub(crate) fn inherit(app: &mut Diagrammer, ui: &mut Ui, label: &str, cells: &[F]) -> bool {
    let shapes = selection(app);
    let Some(o) = app.opened.as_mut() else { return false };
    if shapes.is_empty() {
        return false;
    }
    let r = crate::edit_doc::edit(o, label, true, |tx, eng| {
        let mut sheets = Vec::new();
        for &s in &shapes {
            let sheet = SheetRef::Shape(s);
            for f in cells {
                for a in addrs(tx.doc(), sheet, *f) {
                    if !eng.guarded(tx.doc(), sheet, &a) {
                        tx.inherit_cell(sheet, &a)?;
                    }
                }
            }
            sheets.push(sheet);
        }
        eng.recalc(tx, &sheets)?;
        Ok(())
    });
    ui.repass();
    r.is_ok()
}

// ---------------------------------------------------------------- colours

/// JFLib's fixed colours (FM-08 "Standardfarben"): the NetLab palette's family, from dark red
/// to violet
pub(crate) const STANDARD: [u32; 10] = [0x8B1E1E, 0xC62828, 0xE07B24, 0xF2C230, 0x8BC34A, 0x2E7D32, 0x4FB3D9, 0x1F5FA8, 0x1A2B5C, 0x6A3FA0];

/// the colours of the colour menu
pub(crate) struct Palette {
    pub theme: [u32; 10],
    pub variant: Vec<u32>,
}

/// the design of the page shown (its theme part read for the menu)
fn page_theme(app: &Diagrammer) -> Option<(ThemeInfo, u32)> {
    let o = app.opened.as_ref()?;
    let (id, var) = apply::page_design(&o.doc, crate::design_gallery::page_shown(app)?)?;
    let k = apply::find_part(&o.doc, id)?;
    Some((ThemeInfo::parse(&o.doc.themes.get(k)?.data)?, var))
}

/// the colour menu's colours for the page shown
pub(crate) fn palette(app: &Diagrammer) -> Palette {
    match page_theme(app) {
        Some((th, var)) => {
            let c = |i: usize| th.colors[i].unwrap_or(0);
            let mut theme = [0xFFFFFF, 0x000000, c(1), c(0), 0, 0, 0, 0, 0, 0];
            for k in 0..6 {
                theme[4 + k] = c(4 + k);
            }
            let variant = th.variations.get(var as usize).map(|v| v.iter().map(|x| x.unwrap_or(0)).collect()).unwrap_or_default();
            Palette { theme, variant }
        }
        None => {
            let n = diagram_core::render::no_theme_color;
            Palette { theme: [0xFFFFFF, 0x000000, n(1), n(0), n(2), n(3), n(4), n(5), n(6), n(7)], variant: Vec::new() }
        }
    }
}

/// the names of the design colours (the menu's QuickInfos)
pub(crate) fn theme_names() -> [&'static str; 10] {
    [t("Weiß"), t("Schwarz"), t("Hell"), t("Dunkel"), t("Akzent 1"), t("Akzent 2"), t("Akzent 3"), t("Akzent 4"), t("Akzent 5"), t("Akzent 6")]
}

// the THEMEVAL of a design colour column
fn theme_expr(col: u8) -> String {
    String::from(match col {
        0 => "RGB(255,255,255)",
        1 => "RGB(0,0,0)",
        2 => "THEMEVAL(\"Light\")",
        3 => "THEMEVAL(\"Dark\")",
        4 => "THEMEVAL(\"AccentColor\")",
        5 => "THEMEVAL(\"AccentColor2\")",
        6 => "THEMEVAL(\"AccentColor3\")",
        7 => "THEMEVAL(\"AccentColor4\")",
        8 => "THEMEVAL(\"AccentColor5\")",
        _ => "THEMEVAL(\"AccentColor6\")",
    })
}

/// the formula a pick writes (see the file header)
pub(crate) fn color_formula(app: &Diagrammer, pick: ColorPick, rgb: u32) -> String {
    let fixed = |c: u32| format!("THEMEGUARD(RGB({},{},{}))", (c >> 16) & 255, (c >> 8) & 255, c & 255);
    match pick {
        ColorPick::Theme(col, 0) => format!("THEMEGUARD({})", theme_expr(col)),
        ColorPick::Theme(col, shade) => {
            let p = palette(app);
            let pct = color_shades(p.theme[col as usize % 10])[(shade as usize - 1).min(4)].0;
            format!("THEMEGUARD(MSOTINT({},{pct}))", theme_expr(col))
        }
        ColorPick::Variant(k) => format!("THEMEGUARD(THEMEVAL(\"VariantColor{}\"))", k + 1),
        ColorPick::Standard(_) | ColorPick::Recent(_) => fixed(rgb),
    }
}

/// remember a colour as used last
pub(crate) fn used(app: &mut Diagrammer, rgb: u32) {
    let r = &mut app.format.recent;
    r.retain(|c| *c != rgb);
    r.insert(0, rgb);
    r.truncate(10);
}

/// the colour menu inside an open flyout -> the formula picked
pub(crate) fn color_menu(app: &mut Diagrammer, ui: &mut Ui, id: u32, current: Option<u32>) -> Option<String> {
    let p = palette(app);
    let names = theme_names();
    let recent = app.format.recent.clone();
    let m = ColorMenu {
        theme: &p.theme,
        theme_names: &names,
        variant: &p.variant,
        standard: &STANDARD,
        recent: &recent,
        captions: [t("Designfarben"), t("Variantenfarben"), t("Standardfarben"), t("Zuletzt verwendet")],
        current,
    };
    let (pick, rgb) = ui.color_menu(id, &m)?;
    used(app, rgb);
    Some(color_formula(app, pick, rgb))
}
