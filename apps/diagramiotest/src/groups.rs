// groups -- (JD21) groups, boundary members and Shape ändern of diagram_core (groups, structure::edge,
// replace) on a drawing, written as .vsdx and read back, and the drawing for the Visio counter-check
//
//   check(h)          group "groups": the Group Properties cells (SelectMode 2, DisplayMode 1,
//                     IsDropTarget, IsSnapTarget, IsTextEditTarget, DontMoveChildren), IsDropSource
//                     and EventDblClick OPENGROUPWIN() written and read back; a container's boundary
//                     members (Relationships types 7 .. 10) read back on their edges; the Change
//                     Shape Behavior cells of a master and shapes replaced by its instances (text,
//                     shape data, glue) read back
//   write(dir)        --write-jd21-groups DIR: jd21-gruppen.vsdx (pages "Shape ändern", "Gruppe",
//                     "Randmitglieder") for the user's check in Visio
//
// Everything is built here; nothing comes from Visio.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::glue::{self, Target};
use diagram_core::groups::{self, Props, SelectMode};
use diagram_core::ops::{self, Basic};
use diagram_core::structure::{self, edge, Edge};
use diagram_core::{replace, CellAddr, DocKind, Document, EditError, MasterRef, Owner, PageRef, RowKey, ShapeRef, SheetRef, Text, Transaction, Unit, Value};
use diagram_io::vsdx::{self, Measure, WriteOptions};

use crate::H;

struct Built {
    doc: Document,
    // Shape ändern: the master, the shapes made by replacing, the connector
    master: MasterRef,
    replaced: Vec<ShapeRef>,
    conn: ShapeRef,
    // Gruppe: the group, the shape that may be dropped into it
    group: ShapeRef,
    source: ShapeRef,
    // Randmitglieder: the container, its members on the right and the top edge
    container: ShapeRef,
    edges: Vec<(ShapeRef, Edge)>,
}

const PROPS: Props = Props { select_mode: SelectMode::MembersFirst, display_mode: 1, drop_target: true, snap_target: false, text_edit_target: false, dont_move_children: true };

fn rect(tx: &mut Transaction, eng: &mut Engine, owner: Owner, b: [f64; 4]) -> Result<ShapeRef, EditError> {
    ops::new_shape(tx, eng, owner, Basic::Rectangle, [b[0], b[1]], [b[2], b[3]])
}

fn label(tx: &mut Transaction, eng: &mut Engine, owner: Owner, at: [f64; 2], text: &str) -> Result<(), EditError> {
    let s = ops::new_shape(tx, eng, owner, Basic::TextBox, [at[0], at[1] - 0.5], [at[0] + 9.0, at[1]])?;
    ops::set_text(tx, eng, s, Text::plain(text)).map(|_| ())
}

// the master "Kreis": an ellipse 1.6 x 0.9 in with four connection points, the shape data "Kosten"
// and the cells of the Change Shape Behavior section (shape data kept as the master has it: no;
// User.Typ copied by ReplaceCopyCells)
fn master(tx: &mut Transaction) -> Result<MasterRef, EditError> {
    let m = tx.add_master("Kreis", "Circle")?;
    let mut eng = Engine::new();
    let s = ops::new_shape(tx, &mut eng, Owner::Master(m), Basic::Ellipse, [0.0, 0.0], [1.6, 0.9])?;
    let sh = SheetRef::Shape(s);
    let sk = tx.add_section(sh, "Connection")?;
    for (i, (x, y)) in [("Width*0", "Height*0.5"), ("Width*1", "Height*0.5"), ("Width*0.5", "Height*0"), ("Width*0.5", "Height*1")].into_iter().enumerate() {
        let rk = tx.add_row(sh, sk, Some(RowKey::Index(i as u32)), None)?;
        let (cx, cy) = (tx.intern("X"), tx.intern("Y"));
        tx.set_formula(sh, &CellAddr::row(sk, rk, cx), x)?;
        tx.set_formula(sh, &CellAddr::row(sk, rk, cy), y)?;
    }
    let z = tx.intern("0");
    tx.set_shape_data(sh, "Kosten", "Kosten", Value::Text(z))?;
    tx.set_user(sh, "Typ", Value::Num(0.0), None)?;
    for (c, v) in [("ReplaceLockShapeData", 0.0), ("ReplaceLockText", 0.0), ("ReplaceLockFormat", 1.0)] {
        let a = tx.single(c);
        tx.set_constant(sh, &a, Value::Num(v), Some(Unit::Bool))?;
    }
    let a = tx.single("ReplaceCopyCells");
    tx.set_formula(sh, &a, "DEPENDSON(User.Typ)")?;
    let mut sheets = alloc::vec![sh];
    sheets.push(SheetRef::Master(m));
    eng.recalc(tx, &sheets)?;
    Ok(m)
}

// an old shape of Shape ändern: text, shape data, User.Typ, a red fill
fn decorate(tx: &mut Transaction, s: ShapeRef, text: &str) -> Result<(), EditError> {
    let sh = SheetRef::Shape(s);
    tx.set_text(s, Some(Text::plain(text)))?;
    let v = tx.intern("Anna");
    tx.set_shape_data(sh, "Besitzer", "Besitzer", Value::Text(v))?;
    let v = tx.intern("12");
    tx.set_shape_data(sh, "Kosten", "Kosten", Value::Text(v))?;
    tx.set_user(sh, "Typ", Value::Num(3.0), None)?;
    let a = tx.single("FillForegnd");
    tx.set_formula(sh, &a, "RGB(230,80,60)")
}

fn container(tx: &mut Transaction, eng: &mut Engine, owner: Owner, b: [f64; 4]) -> Result<ShapeRef, EditError> {
    let c = rect(tx, eng, owner, b)?;
    let s = SheetRef::Shape(c);
    let k = tx.intern("Container");
    tx.set_user(s, "msvStructureType", Value::Text(k), Some(Unit::Str))?;
    tx.set_user(s, "msvSDContainerMargin", Value::Num(0.125), Some(Unit::Inches))?;
    tx.set_user(s, "msvSDContainerResize", Value::Num(1.0), None)?;
    eng.recalc(tx, &[s])?;
    Ok(c)
}

fn build() -> Result<Built, String> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let first = doc.pages().next().ok_or("no page")?;
    let mut eng = Engine::new();
    let e = |e: EditError| format!("{e:?}");
    let mut tx = doc.transaction("Seiten");
    tx.rename_page(first, "Shape ändern", "Change Shape").map_err(e)?;
    let second: PageRef = tx.add_page_like(first, "Gruppe", "Group", false, None).map_err(e)?;
    let third: PageRef = tx.add_page_like(first, "Randmitglieder", "Boundary members", false, None).map_err(e)?;
    let m = master(&mut tx).map_err(e)?;
    let _ = tx.commit();

    // Shape ändern: two rectangles joined by a connector (left: as they are; right: replaced)
    let owner = Owner::Page(first);
    let mut tx = doc.transaction("Shape ändern");
    label(&mut tx, &mut eng, owner, [1.6, 7.6], "Links: Ausgang (Rechtecke mit Text, Daten, Verbinder). Rechts: in JFDiagrammer durch „Kreis“ ersetzt.").map_err(e)?;
    let mut replaced = Vec::new();
    let mut conn = None;
    for (x0, replace_them) in [(0.8, false), (6.3, true)] {
        let a = rect(&mut tx, &mut eng, owner, [x0, 4.5, x0 + 2.0, 5.5]).map_err(e)?;
        let b = rect(&mut tx, &mut eng, owner, [x0 + 2.8, 2.5, x0 + 4.4, 3.4]).map_err(e)?;
        decorate(&mut tx, a, "Antrag").map_err(e)?;
        decorate(&mut tx, b, "Prüfung").map_err(e)?;
        eng.recalc(&mut tx, &[SheetRef::Shape(a), SheetRef::Shape(b)]).map_err(e)?;
        let c = glue::new_connector(&mut tx, &mut eng, owner, ([x0 + 2.0, 5.0], Target::Shape(a)), ([x0 + 3.6, 3.4], Target::Shape(b))).map_err(e)?;
        if replace_them {
            replaced = replace::replace(&mut tx, &mut eng, &[a, b], m, 0).map_err(e)?;
            conn = Some(c);
        }
    }
    let _ = tx.commit();

    // Gruppe: a group with Visio's group cells set, a shape that may be dropped into it
    let owner = Owner::Page(second);
    let mut tx = doc.transaction("Gruppe");
    label(&mut tx, &mut eng, owner, [1.6, 7.6], "Gruppe: Elemente zuerst, Gruppendaten hinten, nimmt abgelegte Shapes, Doppelklick öffnet sie. Das kleine Quadrat wird beim Ablegen hinzugefügt.").map_err(e)?;
    let a = rect(&mut tx, &mut eng, owner, [1.0, 3.0, 3.0, 4.5]).map_err(e)?;
    let b = rect(&mut tx, &mut eng, owner, [3.5, 3.0, 5.5, 4.5]).map_err(e)?;
    ops::set_text(&mut tx, &mut eng, a, Text::plain("Element 1")).map_err(e)?;
    ops::set_text(&mut tx, &mut eng, b, Text::plain("Element 2")).map_err(e)?;
    let g = ops::group(&mut tx, &mut eng, owner, &[a, b]).map_err(e)?;
    groups::set_props(&mut tx, &mut eng, g, &PROPS).map_err(e)?;
    groups::set_double_click_opens(&mut tx, &mut eng, g, true).map_err(e)?;
    let src = rect(&mut tx, &mut eng, owner, [7.0, 3.3, 7.8, 4.1]).map_err(e)?;
    groups::set_drop_source(&mut tx, &mut eng, src, true).map_err(e)?;
    let _ = tx.commit();

    // Randmitglieder: a container, a shape dropped on its right edge and one on its top edge
    let owner = Owner::Page(third);
    let mut tx = doc.transaction("Randmitglieder");
    label(&mut tx, &mut eng, owner, [1.6, 7.6], "Container mit zwei Randmitgliedern (rechts, oben) und einem Mitglied innen.").map_err(e)?;
    let c = container(&mut tx, &mut eng, owner, [1.0, 1.5, 6.0, 5.5]).map_err(e)?;
    let inner = rect(&mut tx, &mut eng, owner, [2.0, 2.5, 3.0, 3.2]).map_err(e)?;
    let right = rect(&mut tx, &mut eng, owner, [5.6, 3.0, 6.4, 3.6]).map_err(e)?;
    let top = rect(&mut tx, &mut eng, owner, [2.5, 5.3, 3.5, 5.9]).map_err(e)?;
    for (s, t) in [(inner, "innen"), (right, "Rand rechts"), (top, "Rand oben")] {
        ops::set_text(&mut tx, &mut eng, s, Text::plain(t)).map_err(e)?;
    }
    structure::settle(&mut tx, &mut eng, owner, &[inner, right, top], true).map_err(e)?;
    let edges = alloc::vec![(right, Edge::Right), (top, Edge::Top)];
    let _ = tx.commit();

    Ok(Built { doc, master: m, replaced, conn: conn.ok_or("no connector")?, group: g, source: src, container: c, edges })
}

fn data(doc: &Document, s: ShapeRef, row: &str) -> Option<String> {
    doc.shape_data(SheetRef::Shape(s)).into_iter().find(|r| r.name() == Some(row)).and_then(|r| r.text("Value").map(String::from))
}

fn by_id(doc: &Document, owner: Owner, id: u32) -> Option<ShapeRef> {
    doc.shape_by_id(owner, id)
}

/// group "groups"
pub(crate) fn check(h: &mut H) {
    let b = match build() {
        Ok(b) => b,
        Err(e) => {
            h.check("groups: build the drawing", false, e);
            return;
        }
    };
    let doc = &b.doc;
    // in the model
    h.check("groups: the group cells as set", groups::props(doc, b.group) == PROPS, format!("{:?}", groups::props(doc, b.group)));
    h.check("groups: Shape ändern made two instances of Kreis", b.replaced.len() == 2 && b.replaced.iter().all(|&s| doc.master_of(s) == Some(b.master)), format!("{:?}", b.replaced));
    let edges_now: Vec<Option<Edge>> = b.edges.iter().map(|&(s, _)| edge::edge_of(doc, s, b.container)).collect();
    h.check("groups: boundary members on their edges", edges_now == b.edges.iter().map(|e| Some(e.1)).collect::<Vec<_>>(), format!("{edges_now:?}"));
    // written and read back
    let bytes = match vsdx::write(doc, &WriteOptions::DEFAULT) {
        Ok(x) => x,
        Err(e) => {
            h.check("groups: write", false, format!("{e}"));
            return;
        }
    };
    let back = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            h.check("groups: read back", false, format!("{e}"));
            return;
        }
    };
    let id = |s: ShapeRef| doc.shape(s).map_or(0, |x| x.id());
    let again = |s: ShapeRef| by_id(&back, s.owner, id(s));
    // the group cells, IsDropSource, the double click
    let g2 = again(b.group);
    h.check("groups: Group Properties read back", g2.is_some_and(|g| groups::props(&back, g) == PROPS), format!("{:?}", g2.map(|g| groups::props(&back, g))));
    h.check("groups: EventDblClick OPENGROUPWIN() read back", g2.is_some_and(|g| groups::double_click_opens(&back, g)), String::new());
    h.check("groups: IsDropSource read back", again(b.source).is_some_and(|s| groups::is_drop_source(&back, s)), String::new());
    // boundary members
    let c2 = again(b.container);
    let mut ok = c2.is_some();
    let mut what = String::new();
    for &(s, e) in &b.edges {
        let got = match (again(s), c2) {
            (Some(s2), Some(c2)) => edge::edge_of(&back, s2, c2),
            _ => None,
        };
        ok &= got == Some(e);
        what.push_str(&format!("{got:?} "));
        let rel_a = structure::rel::formula(doc, s).map(String::from);
        let rel_b = again(s).and_then(|s2| structure::rel::formula(&back, s2).map(String::from));
        ok &= rel_a == rel_b && rel_a.as_deref().is_some_and(|f| f.contains("DEPENDSON(8,") || f.contains("DEPENDSON(9,"));
        what.push_str(&format!("{rel_a:?} / {rel_b:?}; "));
    }
    h.check("groups: boundary members read back (Relationships 4 and 7..10)", ok, what);
    h.check("groups: members of the container read back", c2.is_some_and(|c| structure::members(&back, c).len() == 3), format!("{:?}", c2.map(|c| structure::members(&back, c).len())));
    // Change Shape Behavior of the master
    let m2 = back.masters().find(|&m| back.master(m).and_then(|x| x.header.name_u).is_some_and(|n| back.str(n) == "Circle"));
    let beh = m2.and_then(|m| {
        let slot = back.master(m)?.master_shape()?;
        Some(replace::Behavior::of(&back, SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot })))
    });
    h.check(
        "groups: Change Shape Behavior cells read back",
        beh.as_ref().is_some_and(|x| x.lock_format && !x.lock_text && !x.lock_data && x.copy == ["User.Typ"]),
        format!("{beh:?}"),
    );
    // the replaced shapes: text, shape data, the copied User cell, glue
    let mut ok = true;
    let mut what = String::new();
    for (&s, want) in b.replaced.iter().zip(["Antrag", "Prüfung"]) {
        let Some(s2) = again(s) else {
            ok = false;
            continue;
        };
        let text = back.shape(s2).and_then(|x| x.text()).map(|t| t.chars()).unwrap_or_default();
        let typ = back.user_cells(SheetRef::Shape(s2)).into_iter().find(|r| r.name() == Some("Typ")).and_then(|r| r.num("Value"));
        let good = text.trim_end() == want && data(&back, s2, "Besitzer").as_deref() == Some("Anna") && data(&back, s2, "Kosten").as_deref() == Some("12") && typ == Some(3.0) && back.master_of(s2).is_some();
        ok &= good;
        what.push_str(&format!("{text:?} {:?} {typ:?}; ", data(&back, s2, "Besitzer")));
    }
    let glued = again(b.conn).map(|c| glue::glued(&back, c));
    let want = [Target::Shape(again(b.replaced[0]).unwrap_or(b.replaced[0])), Target::Shape(again(b.replaced[1]).unwrap_or(b.replaced[1]))];
    ok &= glued == Some(want);
    h.check("groups: replaced shapes read back with text, data, copied cells and glue", ok, format!("{what} {glued:?}"));
}

/// --write-jd21-groups DIR: jd21-gruppen.vsdx
pub(crate) fn write(dir: &str) -> i32 {
    let built = match build() {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("build: {e}\n"));
            return 1;
        }
    };
    let bytes = match vsdx::write(&built.doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("write: {e}\n"));
            return 1;
        }
    };
    let path = format!("{dir}/jd21-gruppen.vsdx");
    if !plat::write_file(&path, &bytes) {
        crate::out(&format!("cannot write {path}\n"));
        return 1;
    }
    crate::out(&format!("wrote {path}\n"));
    0
}
