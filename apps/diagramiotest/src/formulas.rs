// formulas -- the formula oracle (JD6): every formula cell of the local files recomputed and
// compared with the value Visio stored, plus timing and growth checks on built drawings
//
//   diagramiotest --check --only formulas            (skips the oracle without the local files)
//   diagramiotest --corpus DIR --only formulas --show 20   also prints 20 differing cells with
//                                                    their formulas (local use only: the files
//                                                    are Microsoft content and stay on this machine)
//   diagramiotest --growth REPS                      the times of the built drawings for 1000 ..
//                                                    16000 shapes, REPS runs each (min, median)
//
// The growth check guards against work that grows with the square of the drawing (a scan of
// every cell per cell): eight times the shapes take at most 32 times as long, the minimum of
// three runs each.  Linear work does not give 8x in practice: the memory hierarchy adds a
// factor of its own -- measured 2026-11-19 on a busy machine (other sessions building), 2000 ->
// 16000 shapes: calculate 13 to 16x, an edit 14x; a plain loop that only resolves every formula cell
// 21x, filling a vector 20x -- and quadratic work would give 64x.  32x lies between with room on
// both sides.  The minimum keeps out what a busy machine adds to one run (a 4000-shape run of
// 30 ms once took 131 ms); the sizes are large enough (13 ms and 170 ms) that a time slice
// does not decide the ratio.
//
// Kinds of cells: own formula (F is an expression), F="Inh" (the formula comes from the master
// shape or a style, V is this sheet's result) and inherited without a cell of its own (the
// sheet's value is the inherited one).  A cell the engine does not compute (an unknown
// function, a glue helper, a font ID) keeps its stored value and counts as marked, not as a
// miss.  Numbers agree within 1e-9 relative, texts exactly (colours in any case).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::{parse, Engine, Keep, Outcome};
use diagram_core::{CellAddr, Document, Formula, SheetRef, Value};
use diagram_io::vsdx::{self, Limits};

use crate::{out, H};

const FILES: [&str; 3] = ["Zeichnung2.vsdx", "Zeichnung 3.vsdx", "Favoriten.vssx"];
const TOL: f64 = 1e-9;

// the folder: --corpus DIR, else the first apps/JFDiagrammer/context/files above the current folder
fn folder(arg: Option<String>) -> Option<String> {
    if let Some(d) = arg {
        return Some(d);
    }
    let mut dir = plat::cwd().replace('\\', "/");
    for _ in 0..12 {
        let cand = format!("{dir}/apps/JFDiagrammer/context/files");
        if plat::fs::read_dir(&cand).is_some() {
            return Some(cand);
        }
        match dir.rsplit_once('/') {
            Some((up, _)) if !up.is_empty() => dir = String::from(up),
            _ => break,
        }
    }
    None
}

#[derive(Default)]
struct Tally {
    total: usize,
    hit: usize,
    miss: usize,
    marked: usize,
}

impl Tally {
    fn add(&mut self, o: &Tally) {
        self.total += o.total;
        self.hit += o.hit;
        self.miss += o.miss;
        self.marked += o.marked;
    }
    fn rate(&self) -> f64 {
        let n = self.hit + self.miss;
        if n == 0 { 100.0 } else { 100.0 * self.hit as f64 / n as f64 }
    }
}

#[derive(Default)]
struct Report {
    // by where the sheet is: page (pages and their shapes), master, style (and document)
    place: [Tally; 3],
    own: Tally,
    inh: Tally,
    inherited: Tally,
    by_func: BTreeMap<String, (usize, usize)>,
    by_cell: BTreeMap<String, (usize, usize)>,
    marks: BTreeMap<String, usize>,
    examples: Vec<String>,
    stored_cells: usize,
    ms: u64,
}

fn funcs_of(text: &str) -> Vec<String> {
    let mut v: Vec<String> = match parse(text) {
        Ok(p) => p.functions().map(|f| String::from(p.func_name(f))).collect(),
        Err(_) => alloc::vec![String::from("(parse)")],
    };
    v.sort();
    v.dedup();
    if v.is_empty() {
        v.push(String::from("(no function)"));
    }
    v
}

fn keep_name(k: &Keep) -> String {
    match k {
        Keep::Parse(e) => format!("parse error {:?}", e.kind),
        Keep::Unknown(n) => format!("unknown {n}"),
        Keep::NotComputed(n) => format!("not computed {n}"),
        Keep::Reference => String::from("unknown section"),
        Keep::Deep => String::from("deep"),
    }
}

fn value_text(doc: &Document, v: Value, e: Option<diagram_core::Sym>) -> String {
    let s = match v {
        Value::Absent => String::from("-"),
        Value::Num(x) => format!("{x}"),
        Value::Text(t) => format!("{:?}", doc.str(t)),
    };
    match e {
        Some(e) => format!("{s} E={}", doc.str(e)),
        None => s,
    }
}

fn oracle(doc: &Document, show: usize) -> Report {
    let mut rep = Report::default();
    let mut eng = Engine::new();
    let t0 = plat::time_us();
    let n = diagram_core::formula::formula_cells(doc).len();
    let t1 = plat::time_us();
    let res = eng.calculate(doc);
    rep.ms = plat::time_us().wrapping_sub(t1) / 1000;
    out(&format!("     ({n} formula cells found in {} ms)\n", t1.wrapping_sub(t0) / 1000));
    for (sheet, addr, o) in &res.cells {
        let r = doc.resolve(*sheet, addr);
        let local = r.cached.filter(|_| r.cached.is_some() && local_cell(doc, *sheet, addr));
        let kind = match (local.map(|c| c.formula), r.origin) {
            (Some(Formula::Expr(_)), _) => 0,
            (Some(Formula::Inherited), _) => 1,
            _ => 2,
        };
        let tally = match kind {
            0 => &mut rep.own,
            1 => &mut rep.inh,
            _ => &mut rep.inherited,
        };
        tally.total += 1;
        let place = match sheet {
            SheetRef::Page(_) => 0,
            SheetRef::Shape(r) if matches!(r.owner, diagram_core::Owner::Page(_)) => 0,
            SheetRef::Shape(_) | SheetRef::Master(_) => 1,
            _ => 2,
        };
        let ftext = match r.formula() {
            Formula::Expr(f) => doc.str(f),
            _ => "",
        };
        let cell = String::from(doc.str(addr.cell));
        match o {
            Outcome::Kept(k) => {
                tally.marked += 1;
                *rep.marks.entry(keep_name(k)).or_default() += 1;
            }
            Outcome::Value(v) => {
                let st = v.stored(&cell);
                let ok = st.matches(r.value(), r.error().map(|e| doc.str(e)), doc.strings(), TOL);
                if ok {
                    tally.hit += 1;
                    rep.place[place].hit += 1;
                } else {
                    tally.miss += 1;
                    rep.place[place].miss += 1;
                }
                for f in funcs_of(ftext) {
                    let e = rep.by_func.entry(f).or_default();
                    e.0 += 1;
                    e.1 += !ok as usize;
                }
                let e = rep.by_cell.entry(cell.clone()).or_default();
                e.0 += 1;
                e.1 += !ok as usize;
                if !ok && rep.examples.len() < show {
                    rep.examples.push(format!("{sheet:?} {cell} [{}] F={ftext} V={} got {st:?}", ["own", "Inh", "inherited"][kind], value_text(doc, r.value(), r.error())));
                }
            }
        }
    }
    rep
}

// whether the sheet itself has a cell at addr
fn local_cell(doc: &Document, sheet: SheetRef, addr: &CellAddr) -> bool {
    doc.sheet(sheet).is_some_and(|s| matches!(s.find(addr), diagram_core::Local::Cell(_)))
}

fn stored_formula_cells(doc: &Document) -> usize {
    let mut n = 0;
    for s in diagram_core::formula::all_sheets(doc) {
        let Some(sh) = doc.sheet(s) else { continue };
        let mut count = |c: &diagram_core::Cell| n += matches!(c.formula, Formula::Expr(_) | Formula::Inherited) as usize;
        sh.cells.iter().for_each(&mut count);
        for sec in &sh.sections {
            sec.cells.iter().for_each(&mut count);
            for r in &sec.rows {
                r.cells.iter().for_each(&mut count);
            }
        }
    }
    n
}

// a built drawing of n shapes: each has Width (a constant), Height = Width*2, a User row with
// THEMEVAL() and colour functions, and PinX = the previous shape's PinX + Width (a chain through
// all shapes, which an edit of the first one recomputes)
fn built(n: u32) -> Document {
    use diagram_core::{Cell, Page, PageHeader, Row, RowKey, SecKey, Section, ShapeData, ShapeKind};
    let mut doc = Document::new(diagram_core::DocKind::Drawing);
    let p = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap_or_else(|_| unreachable!());
    let texts = |doc: &mut Document, s: &str| doc.intern(s);
    let (w, hgt, pin, user, val) = (texts(&mut doc, "Width"), texts(&mut doc, "Height"), texts(&mut doc, "PinX"), texts(&mut doc, "User"), texts(&mut doc, "Value"));
    let f_h = texts(&mut doc, "Width*2");
    let f_user = texts(&mut doc, "IF(LUM(RGB(10,20,30))>120,1,SQRT(Height)+MODULUS(Width,0.3))");
    let row = texts(&mut doc, "Calc");
    for i in 0..n {
        let id = i + 1;
        let mut d = ShapeData::new(ShapeKind::Shape);
        d.sheet.cells.push(Cell::constant(w, Value::Num(1.0 + (i % 7) as f64 * 0.1), None));
        d.sheet.cells.push(Cell { name: hgt, value: Value::Num(0.0), formula: Formula::Expr(f_h), unit: None, error: None, stale: false });
        let fpin = if i == 0 { texts(&mut doc, "Width") } else { texts(&mut doc, &format!("Sheet.{}!PinX+Width", id - 1)) };
        d.sheet.cells.push(Cell { name: pin, value: Value::Num(0.0), formula: Formula::Expr(fpin), unit: None, error: None, stale: false });
        let mut sec = Section::new(SecKey::one(user));
        let mut r = Row::new(RowKey::Named(row));
        r.cells.push(Cell { name: val, value: Value::Num(0.0), formula: Formula::Expr(f_user), unit: None, error: None, stale: false });
        sec.rows.push(r);
        d.sheet.sections.push(sec);
        if let Some(pg) = doc.page_mut(p) {
            let _ = pg.shapes.insert(None, id, d);
        }
    }
    doc
}

// full calculation and an edit of the first shape (recalc of the chain) on n shapes -> times
// in microseconds and whether the last PinX is right
fn growth(n: u32) -> (u64, u64, bool) {
    let mut doc = built(n);
    let mut eng = Engine::new();
    let t0 = plat::time_us();
    let res = eng.calculate(&doc);
    let full = plat::time_us().wrapping_sub(t0);
    let mut tx = doc.transaction("w");
    if res.write(&mut tx).is_err() {
        return (full, 0, false);
    }
    drop(tx.commit());
    let page = doc.pages().next();
    let first = page.and_then(|p| doc.shape_by_id(diagram_core::Owner::Page(p), 1));
    let last = page.and_then(|p| doc.shape_by_id(diagram_core::Owner::Page(p), n));
    let (Some(first), Some(last)) = (first, last) else { return (full, 0, false) };
    let w = doc.intern("Width");
    let addr = CellAddr::single(w);
    let t1 = plat::time_us();
    let mut tx = doc.transaction("w2");
    let ok = tx.set_constant(SheetRef::Shape(first), &addr, Value::Num(2.0), None).is_ok()
        && eng.recalc_cells(&mut tx, &[(SheetRef::Shape(first), addr)]).is_ok();
    drop(tx.commit());
    let inc = plat::time_us().wrapping_sub(t1);
    // PinX of the last shape: the sum of all widths (the first one is 2 now)
    let want: f64 = 2.0 + (1..n).map(|i| 1.0 + (i % 7) as f64 * 0.1).sum::<f64>();
    let got = doc.value_of(SheetRef::Shape(last), "PinX").num().unwrap_or(f64::NAN);
    (full, inc, ok && (got - want).abs() < 1e-6)
}

// `reps` runs of growth(n) -> the minimum times of calculate and edit, every result right, the
// median time of calculate (a typical run under the machine's load, for oracle-time)
fn growth_min(n: u32, reps: u32) -> (u64, u64, bool, u64) {
    let (mut full, mut edit, mut ok) = (Vec::new(), Vec::new(), true);
    for _ in 0..reps.max(1) {
        let (f, i, good) = growth(n);
        full.push(f);
        edit.push(i);
        ok &= good;
    }
    full.sort_unstable();
    edit.sort_unstable();
    (full[0], edit[0], ok, full[full.len() / 2])
}

/// --growth REPS: the times of growth() for 1000 .. 16000 shapes, REPS runs each: minimum and
/// median in microseconds (a diagnosis of the growth check)
pub fn growth_table(reps: u32) -> i32 {
    let reps = reps.clamp(1, 50) as usize;
    for n in [1000u32, 2000, 4000, 8000, 16000] {
        let (mut full, mut edit) = (Vec::new(), Vec::new());
        for _ in 0..reps {
            let (f, i, ok) = growth(n);
            if !ok {
                out(&format!("{n}: wrong result\n"));
                return 1;
            }
            full.push(f);
            edit.push(i);
        }
        full.sort_unstable();
        edit.sort_unstable();
        out(&format!("{n:6} shapes: calculate min {:7} us median {:7} us   edit min {:7} us median {:7} us\n", full[0], full[reps / 2], edit[0], edit[reps / 2]));
    }
    0
}

pub fn run(h: &mut H, arg: Option<String>, show: usize) {
    // (16000 built shapes have 48 000 formula cells: Height, PinX and a User row each)
    // growth: eight times the shapes take at most 32 times as long (see the file header)
    let (f1, i1, ok1, _) = growth_min(2000, 3);
    let (f8, i8, ok8, f8_median) = growth_min(16000, 3);
    let ratio = |a: u64, b: u64| b as f64 / (a.max(1)) as f64;
    out(&format!("     built drawings (the fastest of 3 runs): 2000 shapes {} ms (+ edit {} ms), 16000 shapes {} ms (+ edit {} ms)\n", f1 / 1000, i1 / 1000, f8 / 1000, i8 / 1000));
    h.check("formulas/built", ok1 && ok8, String::from("the chain of PinX through all shapes after an edit of the first"));
    h.check("formulas/growth", ratio(f1, f8) < 32.0 && ratio(i1.max(1000), i8) < 32.0, format!("8x the shapes: {:.1}x the time, edit {:.1}x (quadratic: 64x)", ratio(f1, f8), ratio(i1.max(1000), i8)));
    let Some(dir) = folder(arg) else {
        out("skip formulas/oracle: no apps/JFDiagrammer/context/files above the current folder (--corpus DIR)\n");
        return;
    };
    let mut all = Report::default();
    for f in FILES {
        let Some(bytes) = plat::read_file(&format!("{dir}/{f}")) else {
            out(&format!("skip formulas/oracle: {f} is missing\n"));
            return;
        };
        let doc = match vsdx::read(&bytes, &Limits::VSDX) {
            Ok(l) => l.doc,
            Err(e) => {
                h.check("formulas/oracle", false, format!("{f}: {e}"));
                return;
            }
        };
        let rep = oracle(&doc, show.saturating_sub(all.examples.len()));
        let stored = stored_formula_cells(&doc);
        out(&format!(
            "     {f}: {} ms, own {}/{} ({:.2} %), Inh {}/{} ({:.2} %), inherited {}/{} ({:.2} %), marked {}\n",
            rep.ms, rep.own.hit, rep.own.hit + rep.own.miss, rep.own.rate(), rep.inh.hit, rep.inh.hit + rep.inh.miss, rep.inh.rate(),
            rep.inherited.hit, rep.inherited.hit + rep.inherited.miss, rep.inherited.rate(), rep.own.marked + rep.inh.marked + rep.inherited.marked
        ));
        for i in 0..3 {
            all.place[i].add(&rep.place[i]);
        }
        all.own.add(&rep.own);
        all.inh.add(&rep.inh);
        all.inherited.add(&rep.inherited);
        all.ms += rep.ms;
        all.stored_cells += stored;
        for (k, v) in rep.by_func {
            let e = all.by_func.entry(k).or_default();
            e.0 += v.0;
            e.1 += v.1;
        }
        for (k, v) in rep.by_cell {
            let e = all.by_cell.entry(k).or_default();
            e.0 += v.0;
            e.1 += v.1;
        }
        for (k, v) in rep.marks {
            *all.marks.entry(k).or_default() += v;
        }
        all.examples.extend(rep.examples);
    }
    let mut stored = Tally::default();
    stored.add(&all.own);
    stored.add(&all.inh);
    let mut total = Tally::default();
    total.add(&stored);
    total.add(&all.inherited);
    out(&format!(
        "     all: {} stored formula cells; computed {} of them, hit {} ({:.2} %), marked {}; with inherited ones {} cells, hit {:.2} %; {} ms\n",
        all.stored_cells, stored.hit + stored.miss, stored.hit, stored.rate(), stored.marked, total.total, total.rate(), all.ms
    ));
    let p = &all.place;
    out(&format!(
        "     by place (all kinds): page shapes {}/{} ({:.2} %), masters {}/{} ({:.2} %), styles {}/{} ({:.2} %)\n",
        p[0].hit, p[0].hit + p[0].miss, p[0].rate(), p[1].hit, p[1].hit + p[1].miss, p[1].rate(), p[2].hit, p[2].hit + p[2].miss, p[2].rate()
    ));
    let misses: Vec<String> = all.by_func.iter().filter(|(_, v)| v.1 > 0).map(|(k, v)| format!("{k} {}/{}", v.1, v.0)).collect();
    out(&format!("     misses by function (missed/computed): {}\n", misses.join(", ")));
    let mut cells: Vec<(&String, &(usize, usize))> = all.by_cell.iter().filter(|(_, v)| v.1 > 0).collect();
    cells.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    let cells: Vec<String> = cells.iter().take(30).map(|(k, v)| format!("{k} {}/{}", v.1, v.0)).collect();
    out(&format!("     misses by cell: {}\n", cells.join(", ")));
    let marks: Vec<String> = all.marks.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    out(&format!("     marked: {}\n", marks.join(", ")));
    for e in &all.examples {
        out(&format!("     miss {e}\n"));
    }
    // the rest: text sizes without the app's fonts (estimated), values Visio left stale in
    // inherited cells, FillColor2 of solid theme fills (see the hand-over notes of JD6)
    h.check("formulas/oracle", stored.rate() >= 94.0 && total.rate() >= 97.5, format!("{:.2} % of the stored formula cells, {:.2} % with inherited ones", stored.rate(), total.rate()));
    // the cost of a cell against the built drawings' (machine independent: a loaded Linux
    // runner is several times slower than an idle Windows one), and a coarse ceiling; the
    // three files take about half a second on Windows.  The files run once, so the built
    // drawings count with a typical run too (the median, not the fastest).  The ceiling of
    // 10 s holds for a machine as fast as an idle Windows one (built drawings about 3.5 us a
    // cell) and grows with how much slower the machine is now: on 2026-11-19 a loaded WSL run
    // took 11.3 s at 11.2 us a built cell, while its cells cost only 6.6 times the built ones
    let built_us = f8_median as f64 / 48_000.0;
    let corpus_us = all.ms as f64 * 1000.0 / total.total.max(1) as f64;
    let slower = (built_us / 3.5).max(1.0);
    let ceiling = (10_000.0 * slower) as u64;
    h.check(
        "formulas/oracle-time",
        all.ms < ceiling && corpus_us < 15.0 * built_us.max(0.05),
        format!("{} ms for the three files ({} cells, at most {} ms now): {:.1} us a cell, built drawings {:.1} us", all.ms, total.total, ceiling, corpus_us, built_us),
    );
}
