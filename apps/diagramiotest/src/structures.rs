// structures -- (JD10) containers, lists, a cross-functional flowchart and a callout built the
// way JFDiagrammer builds them (diagram_core::structure with the masters of
// apps/JFDiagrammer/stencils/containers, cff and callouts), written with diagram_io and read back
//
//   check(h)          group "structures": the three stencils build; the drawing's
//                     relationships on both sides (Visio's DEPENDSON form, V 0), the list order,
//                     the flowchart's layout, the callout's tip on its target, the structure
//                     functions; after writing and reading the same relationships; moving the
//                     container, a lane and the callout's target after reading carries members
//                     and callout along
//   write(dir)        --write-gegenprobe DIR also writes JD10-Gegenprobe-Strukturen.vsdx for the
//                     Visio counter-check (move members, move the container, add lanes)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::structure::{self, cff, rel};
use diagram_core::{glue, instance, DocKind, Document, EditError, MasterRef, Owner, PageRef, ShapeRef, SheetRef, Text, Transaction, Value};
use diagram_io::stencil;
use diagram_io::vsdx::{self, Measure, WriteOptions};

use crate::H;

macro_rules! files {
    ($dir:literal: $($f:literal),*) => {
        &[$(($f, include_str!(concat!("../../JFDiagrammer/stencils/", $dir, "/", $f)))),*]
    };
}

const CONTAINERS: &[(&str, &str)] = files!("containers": "stencil.txt", "container.master", "list.master", "list_item.master");
const CFF: &[(&str, &str)] = files!("cff": "stencil.txt", "swimlane.master", "separator.master", "swimlane_vertical.master", "separator_vertical.master", "cff_container.master", "swimlane_list.master", "phase_list.master");
const CALLOUTS: &[(&str, &str)] = files!("callouts": "stencil.txt", "callout_box.master", "callout_rounded.master", "callout_bubble.master", "callout_line.master", "callout_bracket.master", "callout_circle.master");

fn build_stencil(table: &[(&str, &str)]) -> Result<Document, String> {
    let f = |name: &str| table.iter().find(|(n, _)| *n == name).map(|(_, t)| String::from(*t));
    stencil::build(&f, Measure::Metric).map(|b| b.doc).map_err(|e| format!("{e}"))
}

fn master(doc: &Document, name_u: &str) -> Option<MasterRef> {
    doc.masters().find(|&m| doc.master(m).and_then(|x| x.header.name_u).is_some_and(|n| doc.str(n) == name_u))
}

// a master of `src` placed at `at` with its values computed
fn place(tx: &mut Transaction, eng: &mut Engine, src: &Document, name_u: &str, page: PageRef, at: [f64; 2]) -> Result<ShapeRef, EditError> {
    let m = master(src, name_u).ok_or(EditError::NoMaster)?;
    let m = instance::import_master(tx, src, m)?;
    let s = instance::place(tx, page, m, at[0], at[1], 0)?;
    eng.recalc(tx, &[SheetRef::Shape(s)])?;
    Ok(s)
}

fn text(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, t: &str) -> Result<(), EditError> {
    ops::set_text(tx, eng, s, Text::plain(&format!("{t}\n")))
}

fn rect(tx: &mut Transaction, eng: &mut Engine, owner: Owner, a: [f64; 2], b: [f64; 2], t: &str) -> Result<ShapeRef, EditError> {
    let s = ops::new_shape(tx, eng, owner, Basic::Rectangle, a, b)?;
    text(tx, eng, s, t)?;
    Ok(s)
}

/// what the drawing holds
pub(crate) struct Built {
    pub doc: Document,
    pub container: ShapeRef,
    pub members: Vec<ShapeRef>,
    pub list: ShapeRef,
    pub items: Vec<ShapeRef>,
    pub pool: ShapeRef,
    pub lanes: Vec<ShapeRef>,
    pub phases: Vec<ShapeRef>,
    pub steps: Vec<ShapeRef>,
    pub target: ShapeRef,
    pub callout: ShapeRef,
}

pub(crate) fn build() -> Result<Built, String> {
    let st_c = build_stencil(CONTAINERS)?;
    let st_f = build_stencil(CFF)?;
    let st_k = build_stencil(CALLOUTS)?;
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let page = doc.pages().next().ok_or("no page")?;
    let owner = Owner::Page(page);
    let mut eng = Engine::new();
    let e = |e: EditError| format!("{e:?}");
    // a container with two members (style 2), fitted around them
    let mut tx = doc.transaction("Container");
    let m1 = rect(&mut tx, &mut eng, owner, [0.8, 7.0], [2.0, 7.6], "Mitglied A").map_err(e)?;
    let m2 = rect(&mut tx, &mut eng, owner, [2.4, 6.4], [3.6, 7.0], "Mitglied B").map_err(e)?;
    let c = place(&mut tx, &mut eng, &st_c, "Container", page, [2.2, 7.0]).map_err(e)?;
    tx.set_user(SheetRef::Shape(c), "msvSDContainerStyle", Value::Num(2.0), None).map_err(e)?;
    eng.recalc(&mut tx, &[SheetRef::Shape(c)]).map_err(e)?;
    text(&mut tx, &mut eng, c, "Container").map_err(e)?;
    ops::reorder(&mut tx, &[c], ops::Z::Back).map_err(e)?;
    structure::add_members(&mut tx, &mut eng, c, &[m1, m2], None).map_err(e)?;
    structure::fit(&mut tx, &mut eng, c, true).map_err(e)?;
    let _ = tx.commit();
    // a list with three items
    let mut tx = doc.transaction("Liste");
    let l = place(&mut tx, &mut eng, &st_c, "List", page, [6.0, 7.0]).map_err(e)?;
    text(&mut tx, &mut eng, l, "Liste").map_err(e)?;
    let mut items = Vec::new();
    for t in ["Eins", "Zwei", "Drei"] {
        let i = place(&mut tx, &mut eng, &st_c, "List item", page, [6.0, 7.0]).map_err(e)?;
        text(&mut tx, &mut eng, i, t).map_err(e)?;
        items.push(i);
    }
    ops::reorder(&mut tx, &[l], ops::Z::Back).map_err(e)?;
    structure::add_members(&mut tx, &mut eng, l, &items, None).map_err(e)?;
    structure::layout_list(&mut tx, &mut eng, l).map_err(e)?;
    let _ = tx.commit();
    // a horizontal cross-functional flowchart: two lanes, two phases, three steps, connectors
    let mut tx = doc.transaction("Flussdiagramm");
    let at = [0.5, 5.3];
    let pool = place(&mut tx, &mut eng, &st_f, "CFF Container", page, at).map_err(e)?;
    text(&mut tx, &mut eng, pool, "Bestellung").map_err(e)?;
    let ll = place(&mut tx, &mut eng, &st_f, "Swimlane List", page, at).map_err(e)?;
    let pl = place(&mut tx, &mut eng, &st_f, "Phase List", page, at).map_err(e)?;
    tx.set_user(SheetRef::Shape(ll), "msvSDListDirection", Value::Num(2.0), None).map_err(e)?;
    tx.set_user(SheetRef::Shape(pl), "msvSDListDirection", Value::Num(0.0), None).map_err(e)?;
    let mut lanes = Vec::new();
    for t in ["Vertrieb", "Technik"] {
        let s = place(&mut tx, &mut eng, &st_f, "Swimlane", page, at).map_err(e)?;
        text(&mut tx, &mut eng, s, t).map_err(e)?;
        lanes.push(s);
    }
    let mut phases = Vec::new();
    for t in ["Planung", "Umsetzung"] {
        let s = place(&mut tx, &mut eng, &st_f, "Separator", page, at).map_err(e)?;
        text(&mut tx, &mut eng, s, t).map_err(e)?;
        phases.push(s);
    }
    for &s in lanes.iter().chain(phases.iter()).chain([pl, ll, pool].iter()) {
        ops::reorder(&mut tx, &[s], ops::Z::Back).map_err(e)?;
    }
    let pb = structure::shape_box(tx.doc(), pool);
    ops::move_shapes(&mut tx, &mut eng, &[pool], [at[0] - pb[0], at[1] - pb[3]]).map_err(e)?;
    structure::add_members(&mut tx, &mut eng, pool, &[ll, pl], None).map_err(e)?;
    structure::add_members(&mut tx, &mut eng, ll, &lanes, None).map_err(e)?;
    structure::add_members(&mut tx, &mut eng, pl, &phases, None).map_err(e)?;
    let f = cff::of_pool(tx.doc(), pool).ok_or("no flowchart")?;
    cff::layout(&mut tx, &mut eng, &f).map_err(e)?;
    let _ = tx.commit();
    // the steps: in lane 1 / phase 1, lane 2 / phase 1, lane 2 / phase 2
    let mut tx = doc.transaction("Schritte");
    let lb: Vec<[f64; 4]> = lanes.iter().map(|&s| structure::shape_box(tx.doc(), s)).collect();
    let phb: Vec<[f64; 4]> = phases.iter().map(|&s| structure::shape_box(tx.doc(), s)).collect();
    let mid = |b: [f64; 4]| (b[1] + b[3]) * 0.5;
    let x0 = |b: [f64; 4]| b[0] + 0.4;
    let s1 = rect(&mut tx, &mut eng, owner, [x0(phb[0]), mid(lb[0]) - 0.25], [x0(phb[0]) + 1.2, mid(lb[0]) + 0.25], "Angebot").map_err(e)?;
    let s2 = rect(&mut tx, &mut eng, owner, [x0(phb[0]), mid(lb[1]) - 0.25], [x0(phb[0]) + 1.2, mid(lb[1]) + 0.25], "Prüfen").map_err(e)?;
    let s3 = rect(&mut tx, &mut eng, owner, [x0(phb[1]), mid(lb[1]) - 0.25], [x0(phb[1]) + 1.2, mid(lb[1]) + 0.25], "Bauen").map_err(e)?;
    let steps = alloc::vec![s1, s2, s3];
    structure::settle(&mut tx, &mut eng, owner, &steps, true).map_err(e)?;
    // the function field as JFDiagrammer adds it
    for &s in &steps {
        let empty = tx.intern("");
        tx.set_shape_data(SheetRef::Shape(s), "Function", "Funktion", Value::Text(empty)).map_err(e)?;
        let (sec, row, val) = (tx.intern("Property"), tx.intern("Function"), tx.intern("Value"));
        let a = diagram_core::CellAddr::row(diagram_core::SecKey { name: sec, index: 0 }, diagram_core::RowKey::Named(row), val);
        tx.set_formula(SheetRef::Shape(s), &a, "IFERROR(CONTAINERSHEETREF(1,\"Swimlane\")!User.visHeadingText,\"\")").map_err(e)?;
        eng.recalc(&mut tx, &[SheetRef::Shape(s)]).map_err(e)?;
    }
    for w in steps.windows(2) {
        let (pa, pb) = (ops::frame(tx.doc(), w[0]).centre(), ops::frame(tx.doc(), w[1]).centre());
        let k = glue::new_connector(&mut tx, &mut eng, owner, (pa, glue::Target::Shape(w[0])), (pb, glue::Target::Shape(w[1]))).map_err(e)?;
        glue::update(&mut tx, &mut eng, &[k]).map_err(e)?;
    }
    let _ = tx.commit();
    // a callout on a shape
    let mut tx = doc.transaction("Beschriftung");
    let target = rect(&mut tx, &mut eng, owner, [8.0, 6.2], [9.6, 7.0], "Ziel").map_err(e)?;
    let k = place(&mut tx, &mut eng, &st_k, "Callout box", page, [10.4, 7.6]).map_err(e)?;
    text(&mut tx, &mut eng, k, "Hinweis").map_err(e)?;
    structure::set_target(&mut tx, &mut eng, k, Some(target)).map_err(e)?;
    structure::set_anchor(&mut tx, k, [9.1, 6.8]).map_err(e)?;
    structure::aim_callouts(&mut tx, &mut eng, &[k]).map_err(e)?;
    let _ = tx.commit();
    Ok(Built { doc, container: c, members: alloc::vec![m1, m2], list: l, items, pool, lanes, phases, steps, target, callout: k })
}

// every relationship named on one side is named back on the other
fn reciprocal(doc: &Document, owner: Owner) -> Result<usize, String> {
    let mut n = 0;
    for s in rel::related_shapes(doc, owner) {
        for (k, others) in &rel::relations(doc, s).groups {
            let Some(back) = rel::back(*k) else { continue };
            for &o in others {
                n += 1;
                if !rel::relations(doc, o).get(back).contains(&s) {
                    return Err(format!("{:?} names {:?} as {k} without the answer {back}", doc.shape(s).map(|x| x.id()), doc.shape(o).map(|x| x.id())));
                }
            }
        }
    }
    Ok(n)
}

pub(crate) fn check(h: &mut H) {
    for (name, t) in [("containers", CONTAINERS), ("cff", CFF), ("callouts", CALLOUTS)] {
        let r = build_stencil(t);
        h.check(&format!("structures: stencil {name} builds"), r.is_ok(), r.err().unwrap_or_default());
    }
    let b = match build() {
        Ok(b) => b,
        Err(e) => {
            h.check("structures: build the drawing", false, e);
            return;
        }
    };
    let doc = &b.doc;
    let page = doc.pages().next().expect("a page");
    let owner = Owner::Page(page);
    let f = rel::formula(doc, b.container).unwrap_or("");
    h.check("structures: container names its members (DEPENDSON 1)", f.starts_with("SUM(DEPENDSON(1,Sheet.") && structure::members(doc, b.container) == b.members, String::from(f));
    let rv = doc.sym("Relationships").map(|c| doc.resolve(SheetRef::Shape(b.container), &diagram_core::CellAddr::single(c)).num());
    h.check("structures: Relationships V = 0", rv == Some(Some(0.0)), format!("{rv:?}"));
    h.check("structures: members name the container (DEPENDSON 4)", b.members.iter().all(|&m| structure::containers_of(doc, m) == alloc::vec![b.container]), String::new());
    let r = reciprocal(doc, owner);
    h.check("structures: every relationship on both sides", r.as_ref().is_ok_and(|n| *n >= 20), format!("{r:?}"));
    h.check("structures: list items in order (DEPENDSON 2)", structure::items(doc, b.list) == b.items, String::new());
    let ib: Vec<[f64; 4]> = b.items.iter().map(|&i| structure::shape_box(doc, i)).collect();
    h.check("structures: list items stacked top to bottom", ib.windows(2).all(|w| (w[0][1] - w[1][3]).abs() < 1e-6), format!("{ib:?}"));
    let fl = cff::of_pool(doc, b.pool);
    h.check("structures: the flowchart is recognised", fl.is_some_and(|f| f.horizontal && cff::lanes(doc, &f) == b.lanes && cff::phases(doc, &f) == b.phases), format!("{fl:?}"));
    let cs = structure::containers_of(doc, b.steps[2]);
    h.check("structures: a step is in its lane and phase", cs.contains(&b.lanes[1]) && cs.contains(&b.phases[1]), format!("{cs:?}"));
    let func = |d: &Document, s: ShapeRef| d.shape_data(SheetRef::Shape(s)).into_iter().find(|r| r.name() == Some("Function")).and_then(|r| r.text("Value").map(String::from)).unwrap_or_default();
    h.check("structures: the step's Funktion is its lane's title", func(doc, b.steps[0]) == "Vertrieb" && func(doc, b.steps[2]) == "Technik", format!("{} / {}", func(doc, b.steps[0]), func(doc, b.steps[2])));
    h.check("structures: the callout's target (DEPENDSON 6 / 3)", structure::target_of(doc, b.callout) == Some(b.target) && structure::callouts_of(doc, b.target) == alloc::vec![b.callout], String::new());
    let tip = structure::association(doc, b.callout).map(|c| c.at);
    h.check("structures: the callout's tip on the target", tip.is_some_and(|p| (p[0] - 9.1).abs() < 1e-6 && (p[1] - 6.8).abs() < 1e-6), format!("{tip:?}"));
    // write, read
    let bytes = match vsdx::write(doc, &WriteOptions::DEFAULT) {
        Ok(x) => x,
        Err(e) => {
            h.check("structures: write", false, format!("{e}"));
            return;
        }
    };
    let mut back = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            h.check("structures: read back", false, format!("{e}"));
            return;
        }
    };
    let id = |d: &Document, s: ShapeRef| d.shape(s).map_or(0, |x| x.id());
    let bp = back.pages().next().expect("a page");
    let bo = Owner::Page(bp);
    let find = |d: &Document, s: ShapeRef| d.shape_by_id(bo, id(doc, s)).expect("the shape");
    let same = |s: ShapeRef| rel::formula(doc, s).map(String::from) == rel::formula(&back, find(&back, s)).map(String::from);
    let all: Vec<ShapeRef> = rel::related_shapes(doc, owner);
    h.check("structures: Relationships formulas read back unchanged", all.iter().all(|&s| same(s)), format!("{} shapes", all.len()));
    let r2 = reciprocal(&back, bo);
    h.check("structures: read back, both sides", r2 == r, format!("{r2:?}"));
    // after reading: moving the container carries its members, a lane its steps, the target its
    // callout
    let mut eng = Engine::new();
    let (c, m1, lane, s1, t, k) = (find(&back, b.container), find(&back, b.members[0]), find(&back, b.lanes[0]), find(&back, b.steps[0]), find(&back, b.target), find(&back, b.callout));
    let before = |d: &Document, s: ShapeRef| structure::shape_box(d, s);
    let (bm1, bs1, bk) = (before(&back, m1), before(&back, s1), before(&back, k));
    let mut tx = back.transaction("Verschieben");
    let r = (|| {
        for (s, d) in [(c, [1.0, 0.5]), (t, [0.5, 0.0])] {
            let along = structure::carried(tx.doc(), &[s]);
            ops::move_shapes(&mut tx, &mut eng, &along, d)?;
            structure::settle(&mut tx, &mut eng, bo, &[s], true)?;
        }
        // lane 1 below lane 2: the lanes trade places, its step goes along
        let lb = structure::shape_box(tx.doc(), lane);
        let other = structure::shape_box(tx.doc(), find(tx.doc(), b.lanes[1]));
        let d = [0.0, other[1] - lb[1] - 0.3];
        let along = structure::carried(tx.doc(), &[lane]);
        ops::move_shapes(&mut tx, &mut eng, &along, d)?;
        structure::settle(&mut tx, &mut eng, bo, &[lane], true)
    })();
    let _ = tx.commit();
    let am1 = before(&back, m1);
    h.check("structures: the container carries its member after reading", r.is_ok() && (am1[0] - bm1[0] - 1.0).abs() < 1e-6 && (am1[1] - bm1[1] - 0.5).abs() < 1e-6, format!("{r:?} {bm1:?} -> {am1:?}"));
    let ak = before(&back, k);
    h.check("structures: the callout follows its target", (ak[0] - bk[0] - 0.5).abs() < 1e-6, format!("{bk:?} -> {ak:?}"));
    let fl2 = cff::of_pool(&back, find(&back, b.pool));
    let order = fl2.map(|f| cff::lanes(&back, &f)).unwrap_or_default();
    h.check("structures: lanes reordered by moving one", order == alloc::vec![find(&back, b.lanes[1]), lane], format!("{order:?}"));
    let as1 = before(&back, s1);
    h.check("structures: the step moved with its lane", as1[1] < bs1[1] - 0.1 && structure::containers_of(&back, s1).contains(&lane), format!("{bs1:?} -> {as1:?}"));
    h.check("structures: Funktion still its lane's", func(&back, s1) == "Vertrieb", func(&back, s1));
}

pub(crate) fn write(dir: &str) -> i32 {
    let b = match build() {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("build: {e}\n"));
            return 1;
        }
    };
    let bytes = match vsdx::write(&b.doc, &WriteOptions::DEFAULT) {
        Ok(x) => x,
        Err(e) => {
            crate::out(&format!("write: {e}\n"));
            return 1;
        }
    };
    let path = format!("{dir}/JD10-Gegenprobe-Strukturen.vsdx");
    if !plat::write_file(&path, &bytes) {
        crate::out(&format!("cannot write {path}\n"));
        return 1;
    }
    crate::out(&format!("wrote {path}\n"));
    0
}
