// apply -- a design or a variant on a page or on every page (TH-02, TH-04)
//
//   apply(tx, eng, id, part, scope, variant)   the design whose theme part is `part` (its
//                                              scheme number `id`) with a variant 0 .. 3
//   set_variant(tx, eng, scope, variant)       another variant of the design the pages have
//   remove(tx, eng, scope)                     no design ("Kein Design": the index cells 0)
//   page_design(doc, page) -> Some((id, variant))
//   part_id(bytes) / find_part(doc, id)        the scheme number of a theme part, and back
//
// What applying does, in one transaction (one undo step; a live preview is that transaction
// taken back):
// 1. The theme part goes into the document: a part with the same scheme number is replaced,
//    else it is added as /visio/theme/themeN.xml.  Parts of JFLib's own designs that no page
//    or shape names any more are dropped again; parts read from a file (Visio's designs) stay
//    -- they are shown, kept and saved, never shipped.
// 2. Each page of the scope gets the scheme numbers of the part's components in ThemeIndex,
//    ColorSchemeIndex, EffectSchemeIndex, ConnectorSchemeIndex and FontSchemeIndex (the own
//    designs use one number for all; Visio's parts mix them), the variant in
//    VariationColorIndex and VariationStyleIndex, EmbellishmentIndex 0 (the variant's).
// 3. Each shape of these pages (also inside groups) loses its own formatting of the cells a
//    design sets, so that the theme shows -- as Visio does: a design overrides formatting made
//    by hand.  Kept are: cells whose formula is protected (THEMEGUARD or GUARD at the top),
//    cells of a kind the shape locks (LockThemeColors: colours; LockThemeEffects: line, fill
//    and shadow properties; LockThemeFonts: typeface and font style; LockThemeConnectors:
//    the line of a connector), the shape's own index cells while LockThemeIndex is set, and
//    the QuickStyle cells (they choose within any design).  Removing a cell lets the shape
//    inherit again -- from its master or its style.
// 4. The engine forgets the parsed theme parts and recalculates the pages and their shapes.

use alloc::vec::Vec;

use crate::cell::{Formula, Value};
use crate::document::{Document, Owner, PageRef, ShapeRef, SheetRef, Theme};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::render::ThemeInfo;
use crate::sheet::{CellAddr, SecKey};

use super::FIRST_ID;

/// which pages a design goes to
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    /// one page
    Page(PageRef),
    /// every page, foreground and background
    All,
}

/// the index cells that choose the design's parts
pub const INDEX_CELLS: [&str; 5] = ["ThemeIndex", "ColorSchemeIndex", "EffectSchemeIndex", "ConnectorSchemeIndex", "FontSchemeIndex"];

/// the index cells of the variant
pub const VARIANT_CELLS: [&str; 2] = ["VariationColorIndex", "VariationStyleIndex"];

// the cells a design sets, by the lock that keeps them
const COLORS: [&str; 4] = ["LineColor", "FillForegnd", "FillBkgnd", "ShdwForegnd"];
const LINE: [&str; 10] =
    ["LineWeight", "LinePattern", "LineCap", "Rounding", "CompoundType", "LineColorTrans", "BeginArrow", "EndArrow", "BeginArrowSize", "EndArrowSize"];
const EFFECTS: [&str; 12] = [
    "FillPattern", "FillForegndTrans", "FillBkgndTrans", "FillGradientEnabled", "ShdwPattern", "ShdwForegndTrans", "ShapeShdwType",
    "ShapeShdwOffsetX", "ShapeShdwOffsetY", "ShapeShdwScaleFactor", "ShapeShdwBlur", "ShapeShdwObliqueAngle",
];
const CHAR_COLORS: [&str; 1] = ["Color"];
const CHAR_FONTS: [&str; 4] = ["Font", "AsianFont", "ComplexScriptFont", "Style"];

/// the scheme number of a theme part (its vt:themeScheme), None without one
pub fn part_id(data: &[u8]) -> Option<u32> {
    ThemeInfo::parse(data)?.scheme_ids[3]
}

/// the index into Document::themes of the part with this scheme number
pub fn find_part(doc: &Document, id: u32) -> Option<usize> {
    doc.themes.iter().position(|t| part_id(&t.data) == Some(id))
}

fn num(doc: &Document, sheet: SheetRef, name: &str) -> Option<f64> {
    let c = doc.sym(name)?;
    doc.resolve(sheet, &CellAddr::single(c)).num()
}

/// the design a page shows: its ThemeIndex and VariationColorIndex (None: no design)
pub fn page_design(doc: &Document, page: PageRef) -> Option<(u32, u32)> {
    let ps = SheetRef::Page(page);
    let id = num(doc, ps, "ThemeIndex").filter(|v| *v > 0.0 && *v < 65534.0)? as u32;
    let var = num(doc, ps, "VariationColorIndex").filter(|v| (0.0..4.0).contains(v)).unwrap_or(0.0) as u32;
    Some((id, var))
}

fn pages_of(doc: &Document, scope: Scope) -> Vec<PageRef> {
    match scope {
        Scope::Page(p) => alloc::vec![p],
        Scope::All => doc.pages().collect(),
    }
}

// the part goes in: it replaces one with its number, else it is added as themeN.xml
fn install(tx: &mut Transaction, id: u32, data: &[u8]) -> Result<(), EditError> {
    let mut themes = tx.doc().themes.clone();
    match themes.iter().position(|t| part_id(&t.data) == Some(id)) {
        Some(i) => themes[i].data = data.to_vec(),
        None => {
            let name = {
                let doc = tx.doc();
                (1..)
                    .map(|n| alloc::format!("/visio/theme/theme{n}.xml"))
                    .find(|nm| !themes.iter().any(|t| doc.str(t.part).eq_ignore_ascii_case(nm)))
                    .unwrap_or_default()
            };
            let part = tx.intern(&name);
            themes.push(Theme { part, data: data.to_vec() });
        }
    }
    tx.set_themes(themes)
}

// the scheme numbers any page or shape names
fn used_ids(doc: &Document) -> Vec<u32> {
    let mut out = Vec::new();
    for p in doc.pages() {
        let mut sheets = alloc::vec![SheetRef::Page(p)];
        sheets.extend(doc.all_shapes(Owner::Page(p)).map(SheetRef::Shape));
        for s in sheets {
            for n in INDEX_CELLS {
                if let Some(v) = num(doc, s, n).filter(|v| *v > 0.0 && *v < 65534.0) {
                    out.push(v as u32);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

// drop the parts of own designs nobody uses; Page::theme indices follow
fn prune(tx: &mut Transaction) -> Result<(), EditError> {
    let doc = tx.doc();
    let used = used_ids(doc);
    let keep: Vec<bool> = doc.themes.iter().map(|t| part_id(&t.data).is_none_or(|id| id < FIRST_ID || used.contains(&id))).collect();
    if keep.iter().all(|k| *k) {
        return Ok(());
    }
    let mut map = Vec::with_capacity(keep.len());
    let mut n = 0u32;
    for k in &keep {
        map.push(k.then_some(n));
        n += u32::from(*k);
    }
    let fixes: Vec<(PageRef, Option<u32>)> = doc
        .pages()
        .filter_map(|p| {
            let t = doc.page(p)?.theme?;
            Some((p, map.get(t as usize).copied().flatten()))
        })
        .collect();
    let themes: Vec<Theme> = doc.themes.iter().zip(&keep).filter(|(_, k)| **k).map(|(t, _)| t.clone()).collect();
    for (p, t) in fixes {
        tx.set_page_theme(p, t)?;
    }
    tx.set_themes(themes)
}

fn set_num(tx: &mut Transaction, sheet: SheetRef, name: &str, v: f64) -> Result<(), EditError> {
    let addr = tx.single(name);
    if tx.doc().resolve(sheet, &addr).num() == Some(v) && tx.doc().sheet(sheet).is_some_and(|s| s.cells.iter().any(|c| c.name == addr.cell)) {
        return Ok(());
    }
    tx.force_constant(sheet, &addr, Value::Num(v), None)
}

// whether a shape's line style chain has the style "Connector"
fn is_connector(doc: &Document, s: ShapeRef) -> bool {
    let Some(sh) = doc.shape(s) else { return false };
    let mut cur = sh.header().styles.line;
    let mut n = 0;
    while let (Some(id), true) = (cur, n < 32) {
        let Some(st) = doc.style_by_id(id).and_then(|r| doc.style(r)) else { break };
        if st.header.name_u.is_some_and(|nm| doc.str(nm) == "Connector") {
            return true;
        }
        cur = st.header.based_on.line;
        n += 1;
    }
    false
}

// the shape's own formatting of themed cells gives way (see the module comment)
fn reset_shape(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(s);
    let locks = eng.locks(tx.doc(), sheet);
    let locked = |n: &str| locks.contains(&n);
    let connector = is_connector(tx.doc(), s);
    let mut names: Vec<&str> = Vec::new();
    if !locked("LockThemeColors") {
        names.extend(COLORS);
    }
    let line_lock = if connector { "LockThemeConnectors" } else { "LockThemeEffects" };
    if !locked(line_lock) {
        names.extend(LINE);
    }
    if !locked("LockThemeEffects") {
        names.extend(EFFECTS);
    }
    if !locked("LockThemeIndex") {
        names.extend(INDEX_CELLS);
        names.extend(VARIANT_CELLS);
        names.push("EmbellishmentIndex");
    }
    let mut addrs: Vec<CellAddr> = Vec::new();
    {
        let doc = tx.doc();
        let Some(sh) = doc.sheet(sheet) else { return Ok(()) };
        for c in &sh.cells {
            if names.contains(&doc.str(c.name)) && c.formula != Formula::Inherited {
                addrs.push(CellAddr::single(c.name));
            }
        }
        let mut char_names: Vec<&str> = Vec::new();
        if !locked("LockThemeColors") {
            char_names.extend(CHAR_COLORS);
        }
        if !locked("LockThemeFonts") {
            char_names.extend(CHAR_FONTS);
        }
        if let Some(ch) = doc.sym("Character") {
            for sec in sh.sections.iter().filter(|x| x.key().name == ch) {
                for row in &sec.rows {
                    for c in &row.cells {
                        if char_names.contains(&doc.str(c.name)) && c.formula != Formula::Inherited {
                            addrs.push(CellAddr::row(SecKey::one(ch), row.key, c.name));
                        }
                    }
                }
            }
        }
    }
    for a in addrs {
        if eng.theme_guarded(tx.doc(), sheet, &a) {
            continue;
        }
        tx.inherit_cell(sheet, &a)?;
    }
    Ok(())
}

// every sheet of the pages: the pages and all their shapes
fn sheets_of(doc: &Document, pages: &[PageRef]) -> Vec<SheetRef> {
    let mut v = Vec::new();
    for &p in pages {
        v.push(SheetRef::Page(p));
        v.extend(doc.all_shapes(Owner::Page(p)).map(SheetRef::Shape));
    }
    v
}

fn finish(tx: &mut Transaction, eng: &mut Engine, pages: &[PageRef]) -> Result<Vec<SheetRef>, EditError> {
    eng.themes_changed();
    let sheets = sheets_of(tx.doc(), pages);
    eng.recalc(tx, &sheets)?;
    Ok(sheets)
}

/// apply the design whose theme part is `part` (scheme number `id`) with variant 0 .. 3 to
/// the pages of `scope` -> the sheets recalculated
pub fn apply(tx: &mut Transaction, eng: &mut Engine, id: u32, part: &[u8], scope: Scope, variant: u32) -> Result<Vec<SheetRef>, EditError> {
    if id == 0 || id >= 65534 {
        return Err(EditError::Invalid);
    }
    install(tx, id, part)?;
    // each component by its own number (Visio's parts mix them: colours 41, format 37 ...);
    // a component without one, or with 0, is taken from any part with the theme's number
    let ids = crate::formula::Theme::parse(part).scheme_ids().map(|i| i.filter(|v| *v != 0 && *v < 65534).unwrap_or(id));
    let pages = pages_of(tx.doc(), scope);
    let variant = variant.min(3) as f64;
    for &p in &pages {
        let ps = SheetRef::Page(p);
        for (n, i) in INDEX_CELLS.iter().zip(ids) {
            set_num(tx, ps, n, i as f64)?;
        }
        for n in VARIANT_CELLS {
            set_num(tx, ps, n, variant)?;
        }
        set_num(tx, ps, "EmbellishmentIndex", 0.0)?;
        let shapes: Vec<ShapeRef> = tx.doc().all_shapes(Owner::Page(p)).collect();
        for s in shapes {
            reset_shape(tx, eng, s)?;
        }
    }
    prune(tx)?;
    finish(tx, eng, &pages)
}

/// another variant of the design each page of `scope` shows (pages without one stay)
pub fn set_variant(tx: &mut Transaction, eng: &mut Engine, scope: Scope, variant: u32) -> Result<Vec<SheetRef>, EditError> {
    let pages: Vec<PageRef> = pages_of(tx.doc(), scope).into_iter().filter(|p| page_design(tx.doc(), *p).is_some()).collect();
    let variant = variant.min(3) as f64;
    for &p in &pages {
        let ps = SheetRef::Page(p);
        for n in VARIANT_CELLS {
            set_num(tx, ps, n, variant)?;
        }
        let shapes: Vec<ShapeRef> = tx.doc().all_shapes(Owner::Page(p)).collect();
        for s in shapes {
            reset_shape(tx, eng, s)?;
        }
    }
    finish(tx, eng, &pages)
}

/// no design on the pages of `scope`: the index cells 0, themed cells fall back to the
/// root style ("Kein Design")
pub fn remove(tx: &mut Transaction, eng: &mut Engine, scope: Scope) -> Result<Vec<SheetRef>, EditError> {
    let pages = pages_of(tx.doc(), scope);
    for &p in &pages {
        let ps = SheetRef::Page(p);
        for n in INDEX_CELLS {
            set_num(tx, ps, n, 0.0)?;
        }
    }
    prune(tx)?;
    finish(tx, eng, &pages)
}
