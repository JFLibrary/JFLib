// templates -- (JD13) the template build tool (diagram_io::template) on JFDiagrammer's own
// templates: every folder of apps/JFDiagrammer/templates, built in both measures with the
// stencils built from apps/JFDiagrammer/stencils (the sources come with the program: build.rs
// lists every file of both folders in OUT_DIR/jfd_sources.rs)
//
//   templates/build      each template builds in both measures, the blank form first, then its
//                        starter diagrams; the same sources give the same bytes
//   templates/roundtrip  read -> write -> read: the same model and the same bytes, no warning
//   templates/windows    the stencil windows: the template's stencils in order, the first
//                        unfolded, "jfd_<stem>_m.vssx" / "_u.vssx"; ParentWindow names the drawing window
//   templates/cells      the JFD_ cells: template, names, description, category, order, starter,
//                        default (exactly one form a template), registers
//   templates/page       page size, orientation and unit: A4 landscape in mm, Letter landscape in in
//   templates/flowchart  Char.Size 8 pt in Normal, the rule set's part (well-formed, its rules)
//   templates/cff        the starters: a flowchart with pool, both lists, two lanes, one phase,
//                        the right orientation, relationships on both sides
//   templates/starter    the tip shape (JFD_Tip), connectors glued at both ends, nothing stale
//   templates/drawing    to_drawing: DocKind::Drawing, English page names and texts, the
//                        catalogue's cells gone, template and registers kept; written as .vsdx
//
//   diagramiotest --write-templates DIR   every .vstx into a folder (to look at in Visio), and
//                        the first page of each starter diagram as PNG

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::glue::{self, Target};
use diagram_core::structure::{self, cff};
use diagram_core::{CellAddr, DocKind, Document, Owner, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Unit};
use diagram_io::stencil;
use diagram_io::template::{self, BuiltTemplate, StencilWindow};
use diagram_io::vsdx::{self, Limits, Measure, WriteOptions};

use crate::{out, H};

/// (kind "stencils" / "templates", folder, file name, text) of every source file
pub(crate) static SOURCES: &[(&str, &str, &str, &str)] = include!(concat!(env!("OUT_DIR"), "/jfd_sources.rs"));

fn folders(kind: &str) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = SOURCES.iter().filter(|s| s.0 == kind).map(|s| s.1).collect();
    v.dedup();
    v
}

pub(crate) fn files_of(kind: &'static str, folder: &'static str) -> impl Fn(&str) -> Option<String> {
    move |name: &str| {
        // (JD13) the names a master gives relative to its folder: "../net_common/asset.inc" (another
        // folder of the same kind), "../../symbols/network/router.svg" (the shared symbols)
        let (k, f, n) = if let Some(rest) = name.strip_prefix("../../symbols/") {
            let (f, n) = rest.split_once('/')?;
            ("symbols", f, n)
        } else if let Some(rest) = name.strip_prefix("../") {
            let (f, n) = rest.split_once('/')?;
            (kind, f, n)
        } else {
            (kind, folder, name)
        };
        SOURCES.iter().find(|s| s.0 == k && s.1 == f && s.2 == n).map(|s| String::from(s.3))
    }
}

/// every stencil of apps/JFDiagrammer/stencils built for measure m
pub(crate) fn stencils(m: Measure) -> Result<Vec<stencil::Built>, String> {
    let mut v = Vec::new();
    for f in folders("stencils") {
        let files = files_of("stencils", f);
        if files("stencil.txt").is_none() {
            continue;
        }
        v.push(stencil::build(&files, m).map_err(|e| format!("stencils/{f}/{e}"))?);
    }
    Ok(v)
}

/// every template built for measure m with the stencils of m: (folder, forms with their bytes)
pub(crate) fn build_all(m: Measure, st: &[stencil::Built]) -> Vec<(&'static str, Result<Vec<(BuiltTemplate, Vec<u8>)>, String>)> {
    let lookup = |stem: &str| st.iter().find(|b| b.file == stem).map(|b| b.doc.clone());
    folders("templates")
        .into_iter()
        .filter(|f| files_of("templates", f)("template.txt").is_some())
        .map(|f| (f, template::build_vstx(&files_of("templates", f), m, &lookup).map_err(|e| format!("templates/{f}/{e}"))))
        .collect()
}

fn mname(m: Measure) -> &'static str {
    if m == Measure::Us { "us" } else { "metric" }
}

fn page1(doc: &Document) -> Option<PageRef> {
    doc.foreground_pages().next()
}

fn num(doc: &Document, sheet: SheetRef, name: &str) -> Option<f64> {
    let c = doc.sym(name)?;
    doc.resolve(sheet, &CellAddr::single(c)).num()
}

fn user(doc: &Document, sheet: SheetRef, name: &str) -> Option<String> {
    let (u, n, v) = (doc.sym("User")?, doc.sym(name)?, doc.sym("Value")?);
    let c = doc.resolve(sheet, &CellAddr::row(SecKey::one(u), RowKey::Named(n), v));
    if !c.exists() {
        return None;
    }
    Some(crate::stencils::sym_text(doc, c.value()))
}

fn part<'d>(doc: &'d Document, ct: &str) -> Option<&'d [u8]> {
    doc.parts.iter().find(|p| doc.str(p.content_type) == ct).map(|p| p.data.as_slice())
}

/// the folder's stencils as template.txt names them, in order (`stencils:`), and the active one
fn named_stencils(folder: &'static str) -> (Vec<String>, Option<String>) {
    let text = files_of("templates", folder)("template.txt").unwrap_or_default();
    let key = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()));
    let list = key("stencils:").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    (list, key("active:"))
}

fn check_forms(h: &mut H, folder: &'static str, m: Measure, forms: &[(BuiltTemplate, Vec<u8>)], again: &[(BuiltTemplate, Vec<u8>)]) {
    let tag = format!("{folder} {}", mname(m));
    h.check("templates/build", !forms.is_empty() && forms[0].0.starter.is_none() && forms.iter().skip(1).all(|f| f.0.starter.is_some()), format!("{tag}: {} forms ({})", forms.len(), forms.iter().map(|f| f.0.vstx_name()).collect::<Vec<_>>().join(", ")));
    let same = forms.len() == again.len() && forms.iter().zip(again).all(|(a, b)| a.1 == b.1);
    h.check("templates/build", same, format!("{tag}: two builds give the same bytes"));
    let mut defaults = 0;
    let (names, active) = named_stencils(folder);
    for (b, bytes) in forms {
        let name = b.vstx_name();
        let suffix = if m == Measure::Us { "_u.vstx" } else { "_m.vstx" };
        h.check("templates/build", name.ends_with(suffix) && b.measure == m && b.doc.kind == DocKind::Template, format!("{name}: a template of the measure"));
        // read -> write -> read: the same model, the same bytes (read_back), and the built
        // model's pages, masters and shapes as read
        let Some(doc) = crate::stencils::read_back(h, "templates/roundtrip", bytes) else { continue };
        let count = |d: &Document| (d.page_count(), d.masters().count(), d.pages().map(|p| d.all_shapes(Owner::Page(p)).count()).sum::<usize>());
        h.check("templates/roundtrip", count(&b.doc) == count(&doc), format!("{name}: pages, masters, shapes {:?}", count(&doc)));
        // the stencil windows
        let wins = template::stencil_windows(&doc);
        let want: Vec<StencilWindow> = names
            .iter()
            .enumerate()
            .map(|(k, s)| StencilWindow { document: stencil::file_name(s, m), active: active.as_ref().map_or(k == 0, |a| a == s) })
            .collect();
        h.check("templates/windows", wins == want, format!("{name}: {:?}", wins.iter().map(|w| format!("{}{}", w.document, if w.active { " (aufgeklappt)" } else { "" })).collect::<Vec<_>>()));
        let wxml = part(&doc, "application/vnd.ms-visio.windows+xml").map(|d| String::from_utf8_lossy(d).into_owned()).unwrap_or_default();
        let parents_ok = wxml.contains("WindowType='Drawing'") && (names.is_empty() || wxml.matches("ParentWindow='0'").count() == names.len());
        h.check("templates/windows", parents_ok, format!("{name}: a drawing window, the stencil windows under it"));
        // the catalogue's cells
        let i = template::info(&doc);
        let ds = SheetRef::Document;
        let names_ok = i.name.as_ref().is_some_and(|n| n.iter().all(|x| !x.is_empty())) && i.description.as_ref().is_some_and(|n| n.iter().all(|x| !x.is_empty()));
        let starter_ok = match &b.starter {
            None => i.starter.is_none(),
            Some(_) => i.starter.as_ref().is_some_and(|n| n.iter().all(|x| !x.is_empty())),
        };
        h.check(
            "templates/cells",
            i.template == b.template && names_ok && starter_ok && i.category == b.category && i.order == b.order && user(&doc, ds, "JFD_Order").is_some(),
            format!("{name}: {} „{}“, {}, order {}, starter {:?}, registers {:?}", i.template, i.name.as_ref().map_or("", |n| n[0].as_str()), i.category, i.order, i.starter.as_ref().map(|s| s[1].clone()), i.registers),
        );
        defaults += usize::from(i.default);
        // the page: size, orientation and unit of the measure
        if let Some(p) = page1(&doc) {
            let ps = SheetRef::Page(p);
            let (w, hh) = (num(&doc, ps, "PageWidth").unwrap_or(0.0), num(&doc, ps, "PageHeight").unwrap_or(0.0));
            let unit = doc.sym("PageWidth").and_then(|c| doc.page(p).and_then(|pg| pg.sheet.cell(c)).and_then(|c| c.unit));
            let (want_w, want_h, want_u) = if m == Measure::Us { (11.0, 8.5, Unit::Inches) } else { (297.0 / 25.4, 210.0 / 25.4, Unit::Millimeters) };
            let orient = num(&doc, ps, "PrintPageOrientation");
            // (JD13) on paper: a page drawn to scale (the rack, 1:20) is A4 or Letter there
            let paper = num(&doc, ps, "PageScale").zip(num(&doc, ps, "DrawingScale")).filter(|(a, d)| *a > 0.0 && *d > 0.0).map_or(1.0, |(a, d)| a / d);
            let (w, hh) = (w * paper, hh * paper);
            let ok = (w - want_w).abs() < 1e-9 && (hh - want_h).abs() < 1e-9 && unit == Some(want_u) && orient == Some(2.0);
            let pname = doc.page(p).and_then(|pg| pg.header.name).map(|n| String::from(doc.str(n))).unwrap_or_default();
            h.check("templates/page", ok, format!("{name}: „{pname}“ {:.1} x {:.1} mm on paper (1:{}), unit {unit:?}, orientation {orient:?}", w * 25.4, hh * 25.4, 1.0 / paper));
            let page_names = user(&doc, ps, "JFD_Name").is_some_and(|t| t.split('|').count() == 5);
            h.check("templates/page", page_names, format!("{name}: the page's name in five languages (JFD_Name)"));
        }
    }
    h.check("templates/cells", defaults == 1, format!("{tag}: {defaults} form(s) with JFD_Default (want one)"));
}

// the flowchart: 8 pt, the rule set
fn check_flowchart(h: &mut H, forms: &[(BuiltTemplate, Vec<u8>)]) {
    let Some((b, _)) = forms.first() else { return };
    let doc = &b.doc;
    let size = doc.style_by_name_u("Normal").and_then(|r| {
        let (ch, sz) = (doc.sym("Character")?, doc.sym("Size")?);
        doc.resolve(SheetRef::Style(r), &CellAddr::row(SecKey::one(ch), RowKey::Index(0), sz)).num()
    });
    h.check("templates/flowchart", size.is_some_and(|s| (s * 72.0 - 8.0).abs() < 1e-9), format!("{}: Normal Char.Size {:?} pt", b.vstx_name(), size.map(|s| s * 72.0)));
    let v = part(doc, "application/vnd.ms-visio.validation+xml").map(|d| String::from_utf8_lossy(d).into_owned());
    let rel = doc.parts.iter().any(|p| doc.str(p.rel_type) == "http://schemas.microsoft.com/visio/2010/relationships/validation");
    let rules = v.as_ref().map_or(0, |t| t.matches("<Rule ").count());
    let well = v.as_deref().is_some_and(xml_ok);
    let shape = v.as_deref().is_some_and(|t| t.contains("<RuleSet ID='1' NameU='Flowchart' Name='Flussdiagramm'") && t.contains("<RuleFilter>") && t.contains("<RuleTest>"));
    h.check("templates/flowchart", rel && well && shape && rules >= 4, format!("{}: validation.xml, {rules} rules, well-formed {well}", b.vstx_name()));
    // through a file: the part and its relationship from the main part
    let bytes = vsdx::write(doc, &WriteOptions::DEFAULT).unwrap_or_default();
    let back = vsdx::read(&bytes, &Limits::VSDX).ok();
    let linked = back.as_ref().is_some_and(|l| l.doc.rels.iter().any(|r| l.doc.str(r.kind) == "http://schemas.microsoft.com/visio/2010/relationships/validation" && l.doc.str(r.source) == "/visio/document.xml"));
    h.check("templates/flowchart", linked, String::from("the main part names validation.xml"));
}

// a well-formed XML part (elements balanced, the root closed): a plain check, no namespaces
fn xml_ok(t: &str) -> bool {
    let mut open: Vec<&str> = Vec::new();
    let mut rest = t;
    let mut roots = 0;
    while let Some(i) = rest.find('<') {
        let tail = &rest[i + 1..];
        let Some(j) = tail.find('>') else { return false };
        let tag = &tail[..j];
        rest = &tail[j + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            if open.pop() != Some(name.trim()) {
                return false;
            }
            continue;
        }
        if open.is_empty() {
            roots += 1;
        }
        let name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        if !tag.ends_with('/') {
            open.push(name);
        }
    }
    open.is_empty() && roots == 1
}

// a starter of the cff template: the flowchart's parts and relationships
fn check_cff(h: &mut H, b: &BuiltTemplate) {
    let name = b.vstx_name();
    let doc = &b.doc;
    let Some(p) = page1(doc) else { return };
    let pools = cff::pools(doc, Owner::Page(p));
    let Some(f) = pools.first() else {
        h.check("templates/cff", false, format!("{name}: no flowchart on the page"));
        return;
    };
    let lanes = cff::lanes(doc, f);
    let phases = cff::phases(doc, f);
    let want_h = b.starter.as_deref() == Some("horizontal");
    h.check(
        "templates/cff",
        pools.len() == 1 && f.lane_list.is_some() && f.phase_list.is_some() && lanes.len() == 2 && phases.len() == 1 && f.horizontal == want_h,
        format!("{name}: {} pool, {} lanes, {} phase, horizontal {}", pools.len(), lanes.len(), phases.len(), f.horizontal),
    );
    // both sides: the pool holds the lists, the lists their items, and the other way round
    let mut both = true;
    for l in [f.lane_list, f.phase_list].into_iter().flatten() {
        both &= structure::members(doc, f.pool).contains(&l) && structure::containers_of(doc, l).contains(&f.pool);
    }
    if let Some(ll) = f.lane_list {
        both &= lanes.iter().all(|&s| structure::list_of(doc, s) == Some(ll));
    }
    if let Some(pl) = f.phase_list {
        both &= phases.iter().all(|&s| structure::list_of(doc, s) == Some(pl));
    }
    h.check("templates/cff", both, format!("{name}: relationships on both sides"));
    // the lanes stacked (horizontal) or side by side, inside the pool
    let pb = structure::shape_box(doc, f.pool);
    let boxes: Vec<[f64; 4]> = lanes.iter().map(|&s| structure::shape_box(doc, s)).collect();
    let inside = boxes.iter().all(|b| b[0] >= pb[0] - 1e-6 && b[2] <= pb[2] + 1e-6 && b[1] >= pb[1] - 1e-6 && b[3] <= pb[3] + 1e-6);
    let laid = boxes.len() == 2 && if want_h { (boxes[0][1] - boxes[1][3]).abs() < 1e-6 } else { (boxes[0][2] - boxes[1][0]).abs() < 1e-6 };
    h.check("templates/cff", inside && laid, format!("{name}: lanes laid out in the pool ({:.0} x {:.0} mm)", (pb[2] - pb[0]) * 25.4, (pb[3] - pb[1]) * 25.4));
    let reg = template::registers(doc);
    h.check("templates/cff", reg == ["cff"], format!("{name}: registers {reg:?}"));
}

// a starter: its tip, its connectors, nothing stale
fn check_starter(h: &mut H, b: &BuiltTemplate) {
    let name = b.vstx_name();
    let doc = &b.doc;
    let Some(p) = page1(doc) else { return };
    let shapes: Vec<ShapeRef> = doc.top_shapes(Owner::Page(p)).collect();
    let tips = shapes.iter().filter(|&&s| user(doc, SheetRef::Shape(s), "JFD_Tip").as_deref() == Some("1")).count();
    h.check("templates/starter", tips == 1, format!("{name}: {} shapes, {tips} tip shape", shapes.len()));
    let connectors: Vec<ShapeRef> = shapes.iter().copied().filter(|&s| glue::is_connector(doc, s)).collect();
    let glued = connectors.iter().all(|&c| glue::glued(doc, c).iter().all(|t| matches!(t, Target::Shape(_))));
    h.check("templates/starter", glued, format!("{name}: {} connectors glued at both ends", connectors.len()));
    // (JD26) 1-D masters glued to connection points by the starter (BeginOn / EndOn): the arrows
    // of the block diagram, the bones and causes of the fishbone
    let point_glued = |s: ShapeRef| glue::glued(doc, s).iter().filter(|t| matches!(t, Target::Point { .. })).count();
    let want = if name.starts_with("block-") {
        Some(6)
    } else if name.starts_with("cause_effect-") {
        Some(8)
    } else {
        None
    };
    if let Some(want) = want {
        let got: usize = shapes.iter().filter(|&&s| glue::is_one_d(doc, s) && !glue::is_connector(doc, s)).map(|&s| point_glued(s)).sum();
        h.check("templates/starter/glue", got == want, format!("{name}: {got} of {want} ends glued to connection points"));
    }
    let res = Engine::new().calculate(doc);
    let mut d2 = doc.clone();
    let mut tx = d2.transaction("check");
    let n = res.write(&mut tx).unwrap_or(usize::MAX);
    h.check("templates/starter", n == 0, format!("{name}: {n} values stale after building"));
}

// to_drawing on the read file, in English
fn check_drawing(h: &mut H, b: &BuiltTemplate, bytes: &[u8]) {
    let name = b.vstx_name();
    let Ok(l) = vsdx::read(bytes, &Limits::VSDX) else { return };
    let mut doc = l.doc;
    let windows = template::stencil_windows(&doc);
    template::to_drawing(&mut doc, 1);
    let ds = SheetRef::Document;
    let p = page1(&doc);
    let pname = p.and_then(|p| doc.page(p)).and_then(|pg| pg.header.name).map(|n| String::from(doc.str(n))).unwrap_or_default();
    let gone = ["JFD_Name", "JFD_Description", "JFD_Category", "JFD_Order", "JFD_Starter", "JFD_Default"].iter().all(|n| user(&doc, ds, n).is_none());
    let kept = user(&doc, ds, "JFD_Template").as_deref() == Some(b.template.as_str()) && template::stencil_windows(&doc) == windows;
    // (JD26) the shapes' own JFD_Text; the parts of a group master inherit theirs from the master
    let own_text = |s: SheetRef| -> bool {
        let (Some(u), Some(n)) = (doc.sym("User"), doc.sym("JFD_Text")) else { return false };
        doc.sheet(s).and_then(|sh| sh.section(SecKey::one(u))).and_then(|sec| sec.row(RowKey::Named(n))).is_some_and(|r| !r.del)
    };
    let texts_gone = p.is_some_and(|p| doc.all_shapes(Owner::Page(p)).all(|s| !own_text(SheetRef::Shape(s))));
    let page_ok = pname == "Page-1" && p.is_some_and(|p| user(&doc, SheetRef::Page(p), "JFD_Name").is_none());
    h.check("templates/drawing", doc.kind == DocKind::Drawing && gone && kept && texts_gone && page_ok, format!("{name}: a drawing, page „{pname}“, cells gone {gone}, kept {kept}"));
    // English texts: the tip, the masters' texts
    if let Some(p) = p {
        let english = doc.all_shapes(Owner::Page(p)).filter_map(|s| doc.shape(s).and_then(|x| x.text()).map(|t| t.chars())).filter(|t| t.starts_with("Tip:") || t == "Function" || t == "Title" || t == "Start" || t == "Check order").count();
        let german = doc.all_shapes(Owner::Page(p)).filter_map(|s| doc.shape(s).and_then(|x| x.text()).map(|t| t.chars())).filter(|t| t.starts_with("Tipp:") || t == "Funktion" || t == "Titel" || t == "Bestellung prüfen").count();
        if b.starter.is_some() {
            h.check("templates/drawing", english > 0 && german == 0, format!("{name}: {english} English texts, {german} German ones"));
        }
    }
    let bytes = vsdx::write(&doc, &WriteOptions::DEFAULT).unwrap_or_default();
    let back = vsdx::read(&bytes, &Limits::VSDX);
    h.check("templates/drawing", back.as_ref().is_ok_and(|l| l.format.kind == DocKind::Drawing), format!("{name}: written as .vsdx ({} KB) and read back", bytes.len() / 1024));
}

pub fn run(h: &mut H) {
    for m in [Measure::Metric, Measure::Us] {
        let st = match stencils(m) {
            Ok(s) => s,
            Err(e) => {
                h.check("templates/build", false, format!("the stencils: {e}"));
                continue;
            }
        };
        let all = build_all(m, &st);
        let again = build_all(m, &st);
        h.check("templates/build", all.len() >= 4, format!("{} {} templates: {}", all.len(), mname(m), all.iter().map(|a| a.0).collect::<Vec<_>>().join(", ")));
        for ((folder, r), (_, r2)) in all.iter().zip(&again) {
            let (forms, forms2) = match (r, r2) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => {
                    h.check("templates/build", false, e.clone());
                    continue;
                }
            };
            check_forms(h, folder, m, forms, forms2);
            if *folder == "flowchart" {
                check_flowchart(h, forms);
            }
            for (b, bytes) in forms.iter().filter(|f| f.0.starter.is_some()) {
                check_starter(h, b);
                if *folder == "cff" {
                    check_cff(h, b);
                }
                check_drawing(h, b, bytes);
            }
            if let Some((b, bytes)) = forms.first() {
                check_drawing(h, b, bytes);
            }
        }
    }
}

/// diagramiotest --write-templates DIR: every form of every template, both measures
pub fn write(dir: &str) -> i32 {
    let mut status = 0;
    for m in [Measure::Metric, Measure::Us] {
        let st = match stencils(m) {
            Ok(s) => s,
            Err(e) => {
                out(&format!("error: {e}\n"));
                return 1;
            }
        };
        for (_, r) in build_all(m, &st) {
            match r {
                Ok(forms) => {
                    for (b, bytes) in forms {
                        let path = format!("{dir}/{}", b.vstx_name());
                        if plat::write_file(&path, &bytes) {
                            out(&format!("wrote {path} ({} KB)\n", bytes.len() / 1024));
                        } else {
                            out(&format!("error: cannot write {path}\n"));
                            status = 1;
                        }
                        // the first page of a starter diagram as a picture
                        let mut doc = b.doc.clone();
                        if let (Some(_), Some(p)) = (&b.starter, page1(&doc)) {
                            let png = format!("{dir}/{}.png", b.vstx_name().trim_end_matches(".vstx"));
                            if !crate::stencil_structs::render_page_at(&mut doc, p, &png, 60.0) {
                                status = 1;
                            }
                        }
                    }
                }
                Err(e) => {
                    out(&format!("error: {e}\n"));
                    status = 1;
                }
            }
        }
    }
    status
}
