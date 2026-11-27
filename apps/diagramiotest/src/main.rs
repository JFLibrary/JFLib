// diagramiotest -- the tests of the diagram codecs (lib/diagram_io), after the pattern of codectest
//
//   diagramiotest --check [--only NAME]   every group, or those whose names start with NAME;
//                                         exit code 1 when a check fails (tests/vsdx.uis runs it)
//   diagramiotest --corpus DIR            the local load tests with the files in DIR (default: the
//                                         first apps/JFDiagrammer/context/files above the current
//                                         folder; skipped when there is none)
//   diagramiotest --write-samples DIR     write the self-built sample files (.vsdx, .vssx, .vsdm ...)
//                                         to DIR, to open them elsewhere
//   diagramiotest --write-golden DIR      (JD4) the golden images of the viewer tests anew
//   diagramiotest --write-drawings DIR    (JD4) the viewer tests' drawings as .vsdx
//   diagramiotest --write-gegenprobe DIR  (JD7) the glue drawing for the Visio counter-check,
//                                         (JD10) JD10-Gegenprobe-Strukturen.vsdx
//   diagramiotest --growth REPS           the formula engine's times on built drawings of 1000 ..
//                                         16000 shapes (min, median of REPS runs)
//   diagramiotest --render-corpus DIR [--width PX]   (JD4) the local files' pages as PNG and SVG
//   diagramiotest --write-export-golden DIR   (JD12) the export's golden images anew
//   diagramiotest --write-exports DIR [--corpus DIR]   (JD12) the self-built drawings and the local
//                                         files as PDF, SVG and PNG (to compare with Visio)
//   diagramiotest --write-stencils DIR   (JD9) the start set's stencils as .vssx (stencils.rs)
//   diagramiotest --write-design-golden DIR   (JD11) the designs' golden images anew
//   diagramiotest --design-probe DIR     (JD11) the own designs on the sample drawing as PNG, and
//                                         one .vsdx a design for the Visio counter-check (designs.rs)
//   diagramiotest --write-stencil-previews DIR   (JD10) the structure stencils' masters as PNG
//                                         (containers, cff, callouts; stencil_structs.rs), (JD13)
//                                         probeblatt.png: the shape stencils (stencil_shapes.rs),
//                                         <folder>-alle.png: every master of each stencil
//                                         (stencil_overview.rs), (JD26) probeblatt-jd26.png: the
//                                         general and business-graphics stencils
//                                         (stencil_business.rs)
//   diagramiotest --write-jd26-gegenprobe DIR   (JD26) the general and business-graphics
//                                         stencils, their templates, a drawing from each and
//                                         every master for the Visio counter-check
//   diagramiotest --write-templates DIR  (JD13) every form of JFDiagrammer's templates as .vstx,
//                                         metric and US (templates.rs)
//   diagramiotest --write-data DIR [--xlsx PATH]   (JD18) jd18-verknuepft.vsdx: a drawing with
//                                         linked data for the Visio counter-check (data.rs)
//   diagramiotest --write-link-fixture DIR   (JD18) jd18-link.vsdx and jd18-link.csv, the fixture
//                                         of JFDiagrammer's data_link.uis and
//                                         data_refresh_conflict.uis (data_fixture.rs)
//   diagramiotest --write-dg DIR         (JD19) jd19-datengrafik.vsdx: data graphics of all four kinds
//                                         and a legend for the Visio counter-check, jd19-dg.vsdx: the
//                                         fixture of JFDiagrammer's data_graphics*.uis (datagraphics.rs)
//   diagramiotest --write-report DIR     (JD19) jd19-bericht.vsdx: shapes with shape data, two
//                                         report definitions and a report table for the Visio
//                                         counter-check (reports.rs); --write-report-fixture DIR
//                                         jd19-geraete.vsdx, the fixture of JFDiagrammer's
//                                         report scripts
//   diagramiotest --write-vdx DIR        (JD16) the self-written .vdx / .vsx files and the .vsdx /
//                                         .vssx JFDiagrammer writes from them (vdx.rs)
//   diagramiotest --write-vss DIR        (JD17) the sample stencil jd17-schablone.vss (three masters
//                                        with geometry formulas; JFDiagrammer's binary_stencil.uis)
//   diagramiotest --write-vss-hostile DIR (JD17) the synthetic .vss files and their hostile
//                                         variants (vss.rs)
//   diagramiotest --write-jd21-ops DIR   (JD21) jd21-vorgaenge.vsdx: input sets of the shape
//                                         operations and JFDiagrammer's results for the Visio
//                                         counter-check (shapeops.rs)
//   diagramiotest --write-vss-overview DIR [--corpus DIR]   (JD17) every master of each local .vss
//                                         on one sheet (<stem>-alle.png), and of each .vssx Visio
//                                         saved from them (<stem>-visio-alle.png; vss.rs)
//   diagramiotest --write-jd21-jumps DIR  (JD21) JD21-Gegenprobe-Linienuebergaenge.vsdx (line
//                                         jumps: a page per style, code, direction ...) and a PNG
//                                         of each page (jumps.rs)
//   diagramiotest --compare-visio-jumps FILE  (JD21v) a drawing Visio saved after laying every
//                                         connector anew: the same laying here, the rows against
//                                         Visio's (jumps.rs)
//   diagramiotest --write-jd21-groups DIR (JD21) jd21-gruppen.vsdx: Shape ändern, a group with its
//                                         behaviour cells, boundary members (groups.rs)
//   diagramiotest --write-jd21-route DIR  (JD21) JD21-Gegenprobe-Routing.vsdx (a page per routing
//                                         style and direction, permeable shapes, a way by hand)
//                                         and a PNG of each page (route.rs)
//
// Groups:
//   samples   self-built files of all six formats: what lands where in the model (fields,
//             extras at their place, parts and relationships kept, nothing recomputed)
//   xml       hostile XML: broken syntax, entities, DOCTYPE, encodings, depth, attributes
//   limits    counts against vsdx::Limits: shapes, cells, rows, sections, formula and text
//             length; large drawings within the limits in reasonable time
//   refs      cycles of styles, masters and background pages; references that name nothing
//   rels      wrong relationships, content types, parts used twice, macros, wrong IDs
//   zip       ZIP bombs and lying archives through lib/archive (ratio, sizes, counts)
//   corpus    the local files (Zeichnung2, Zeichnung 3, Favoriten): counts against FEATURES.md
//             1.6; nothing is copied, only numbers are printed
//   render    (JD4, render.rs, drawings.rs) the viewer: display lists of self-built drawings,
//             golden images (tests/golden) through render2d's CPU path, the local pages drawn
//   export    (JD12, export.rs) PDF through pdftest's reader, SVG read back by img_svg and
//             compared with the pixels, PNG / JPEG golden images and round trips, transparency,
//             the page selection, the shapes' links
//   formulas  the formula oracle (JD6): every formula cell of the local files recomputed and
//             compared with the stored value (--show N prints N misses, locally)
//   editview  (JD7, edit_view.rs) PageView::update against a page built anew, Metrics' text
//             sizes against the drawn text, the time an edit of one of 1000 shapes takes
//   designs   (JD11, designs.rs) the own designs: their theme parts, the sample drawing written
//             and read back, golden images (tests/golden/design-*.png), applying with undo,
//             foreign parts kept, Visio's mixed scheme numbers, THEMEGUARD and locks
//   edit      (JD7, gegenprobe.rs) static and dynamic glue through ops and glue, moved, written
//             and read back: the Connect entries and glue formulas stay, ends follow again
//   stencils  (JD9, stencils.rs) the stencil build tool: the start set, the SVG import, faults
//             (JD10, stencil_structs.rs) the structure stencils: containers, lists, lanes, callouts
//             (JD13, stencil_shapes.rs) the shape stencils: blocks, arrows, flowchart, connectors
//             (1-D masters, glue)
//             (JD26, stencil_business.rs) stencils/business: the general and business-graphics
//             stencils: parts in five languages, counts and values from shape data and control
//             points, the fishbone's glue
//   structures  (JD10, structures.rs) a container, a list, a cross-functional flowchart and a
//             callout built as JFDiagrammer builds them: relationships on both sides, written and
//             read back unchanged, members, steps and callouts carried after reading
//   templates (JD13, templates.rs) JFDiagrammer's templates built from their sources in both
//             measures: round trip, stencil windows, JFD_ cells, pages, the rule set, the
//             starters (cross-functional flowcharts, connectors, tips), to_drawing
//   data      (JD18, data.rs) external data: recordsets and connections written and read back,
//             a package as Visio writes one, a refresh saved, parts not understood kept, hostile
//             data parts
//   datagraphics  (JD19, datagraphics.rs) data graphics: the callout masters of
//             stencils/datagraphics, a graphic of all four kinds and a legend written and read
//             back with the values they show, a refresh reaching the callouts, removal; a graphic
//             stored the way Visio stores one read, kept and applied
//   reports   (JD19, reports.rs) shape report definitions in the document sheet (visPropRptN)
//             written and read back, one as Visio writes it, a broken one kept; the report table
//             shape's marks and layer; the report run on the read drawing
//   vdx       (JD16, vdx.rs) the XML formats of 2003-2010: the self-written drawing, stencil and
//             bitmap read into the model and written as .vsdx / .vssx, hostile files and
//             limits; macro projects passed through unchanged; document properties
//   shapeops  (JD21, shapeops.rs) the shape operations on rectangles, an ellipse, a turned
//             rectangle, an ArcTo shape and a shape with a hole: arcs stay arcs, the results
//             written and read back with the same rows, formulas and frames, stretched results
//   vss       (JD17, vss.rs) the binary stencils of 2003-2010: compound files (cfbkit.rs) and
//             VisioDocument streams (vsskit.rs) built here, hostile variants of both, cut and
//             mutated files; the local .vss of SWDB_DE read, written as .vssx and read back, and
//             drawn against Visio's .vssx of them (skipped without the files)
//   jumps     (JD21, jumps.rs) line jumps: crossing connectors per style, code, direction, size;
//             the jumps in the display list, the cells written and read back (no geometry), the
//             SVG and PNG export
//   groups    (JD21, groups.rs) the Group Properties cells, IsDropSource and the double click
//             read back, boundary members (Relationships 7 .. 10), Change Shape Behavior cells
//             and shapes replaced by a master's instances (text, data, copied cells, glue)
//   route     (JD21, route.rs) routing styles with directions, permeable shapes, a way by hand:
//             the sides the ways use, the routing cells and ways written and read back, routed
//             again after reading; the local corpus' connectors (right angles, ConFixedCode 0, 3 .. 6)
//   pointglue (JD21 glue, pointglue.rs) a control point glued to a connection point, new
//             connection points and their kinds, points made at a vertex and a handle, an end on
//             a guide: Connect entries, rows and formulas written and read back, following after
//             reading
//
// Why an app of its own and not codectest: codectest is the image codecs' runner (with the
// assembly-parity protocol and every image codec linked); the diagram codecs grow with JD5
// (round trip), JD6 (formula oracle) and JD12 (exports), and their tests belong next to them.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

mod build;
mod cfbkit;
mod corpus;
// (JD18) external data: connections, recordsets, linked shapes
mod data;
mod data_fixture;
// (JD19) data graphics
mod datagraphics;
mod designs;
mod drawings;
mod dump;
mod edit_view;
mod export;
mod formulas;
mod gegenprobe;
mod hostile;
mod hostile_write;
// (JD21) line jumps
mod jumps;
// (JD21 glue) control points, connection points, vertices, handles and guides glued
mod pointglue;
mod pkg;
// (JD12) pdftest's independent PDF reader checks the exported PDFs
#[path = "../../pdftest/src/reader.rs"]
mod pdfreader;
mod render;
// (JD19) shape reports
mod reports;
mod roundtrip;
// (JD21) routing styles, permeable shapes, a way by hand
mod route;
mod samples;
// (JD21) the shape operations
mod shapeops;
// (JD21) groups, boundary members, Shape ändern
mod groups;
mod stencil_business;
mod stencil_network;
mod stencil_overview;
mod stencil_shapes;
mod stencil_structs;
mod stencils;
mod structures;
mod svgdump;
mod symbol_sheet;
mod templates;
mod vdx;
mod vdx_sample;
mod vss;
mod vss_sample;
mod vsskit;
mod zipkit;

/// a refusal must come within this many microseconds
pub const QUICK_US: u64 = 1_000_000;

/// the check harness
pub struct H {
    pub ok: u32,
    pub failed: u32,
    only: Option<String>,
}

impl H {
    pub fn on(&self, name: &str) -> bool {
        self.only.as_ref().is_none_or(|o| name.starts_with(o.as_str()))
    }
    pub fn check(&mut self, name: &str, cond: bool, what: String) {
        if cond {
            self.ok += 1;
            out(&format!("ok   {name}: {what}\n"));
        } else {
            self.failed += 1;
            out(&format!("FAIL {name}: {what}\n"));
        }
    }
    /// f() must give `want` within QUICK_US
    pub fn quick<T: PartialEq + core::fmt::Debug>(&mut self, name: &str, want: T, f: impl FnOnce() -> T) {
        let t0 = plat::time_us();
        let r = f();
        let us = plat::time_us().wrapping_sub(t0);
        let what = format!("{r:?} (want {want:?}) in {} ms", us / 1000);
        self.check(name, r == want && us < QUICK_US, what);
    }
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    if let Some(dir) = arg(&args, "--write-samples") {
        return samples::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-golden") {
        return render::write_golden(&dir);
    }
    if let Some(dir) = arg(&args, "--write-drawings") {
        return render::write_drawings(&dir);
    }
    if let Some(dir) = arg(&args, "--render-corpus") {
        let width = arg(&args, "--width").and_then(|w| w.parse().ok()).unwrap_or(2000);
        return render::render_corpus(&dir, arg(&args, "--corpus"), width);
    }
    if let Some(dir) = arg(&args, "--write-export-golden") {
        return export::write_golden(&dir);
    }
    if let Some(dir) = arg(&args, "--write-exports") {
        return export::write_exports(&dir, arg(&args, "--corpus"));
    }
    if let Some(dir) = arg(&args, "--write-stencils") {
        return stencils::write(&dir);
    }
    if let Some(reps) = arg(&args, "--growth") {
        return formulas::growth_table(reps.parse().unwrap_or(5));
    }
    if let Some(dir) = arg(&args, "--symbol-sheet") {
        // (JD13) every SVG of a folder on one sheet (the shared network symbols)
        let path = args.iter().position(|a| a == "--symbol-sheet").and_then(|i| args.get(i + 2)).cloned().unwrap_or_else(|| String::from("symbols.png"));
        return symbol_sheet::write(&dir, &path, arg(&args, "--cols").and_then(|c| c.parse().ok()).unwrap_or(6));
    }
    if let Some(dir) = arg(&args, "--write-network-gegenprobe") {
        // (JD13) a network plan and a rack drawing for a look in Visio
        return stencil_network::gegenprobe(&dir);
    }
    if let Some(dir) = arg(&args, "--write-network-previews") {
        // (JD13) the network symbols and a rack with devices
        return stencil_network::previews(&dir);
    }
    if let Some(dir) = arg(&args, "--write-stencil-previews") {
        // (JD26) probeblatt-jd26.png: the general and business-graphics stencils
        return stencil_structs::previews(&dir).max(stencil_shapes::previews(&dir)).max(stencil_overview::write_all(&dir)).max(stencil_business::previews(&dir));
    }
    if let Some(dir) = arg(&args, "--write-jd26-gegenprobe") {
        // (JD26) the stencils, templates and drawings for the Visio counter-check
        return stencil_business::gegenprobe(&dir);
    }
    if let Some(dir) = arg(&args, "--write-gegenprobe") {
        // (JD7) the glue drawing, (JD10) the structures drawing
        return gegenprobe::write(&dir).max(structures::write(&dir));
    }
    if let Some(dir) = arg(&args, "--write-design-golden") {
        return designs::write_golden(&dir);
    }
    if let Some(dir) = arg(&args, "--design-probe") {
        return designs::probe(&dir);
    }
    if let Some(dir) = arg(&args, "--write-templates") {
        return templates::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-vdx") {
        // (JD16) the self-written .vdx / .vsx and what JFDiagrammer writes from them
        return vdx::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-link-fixture") {
        // (JD18) the drawing and the text file of JFDiagrammer's data scripts
        return data_fixture::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-data") {
        // (JD18) the drawing with linked data for the Visio counter-check
        return data::write_check(&dir, arg(&args, "--xlsx").as_deref());
    }
    if let Some(dir) = arg(&args, "--write-dg") {
        // (JD19) the data graphics for the Visio counter-check and JFDiagrammer's scripts
        return datagraphics::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-report-fixture") {
        // (JD19) the drawing of JFDiagrammer's report scripts
        return reports::write_fixture(&dir);
    }
    if let Some(dir) = arg(&args, "--write-report") {
        // (JD19) the drawing with report definitions and a report table for the Visio counter-check
        return reports::write_check(&dir);
    }
    if let Some(dir) = arg(&args, "--write-vss-overview") {
        // (JD17) every master of each local .vss on one sheet
        return vss::write_overview(&dir, arg(&args, "--corpus"));
    }
    if let Some(dir) = arg(&args, "--write-vss") {
        // (JD17) the sample stencil jd17-schablone.vss (JFDiagrammer's tests/binary_stencil.uis)
        return vss_sample::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-vss-hostile") {
        // (JD17) the synthetic .vss files and their hostile variants
        return vss::write_hostile(&dir);
    }
    if let Some(dir) = arg(&args, "--write-jd21-jumps") {
        // (JD21) the line jumps drawing for the Visio counter-check, a PNG per page
        return jumps::write(&dir);
    }
    if let Some(file) = arg(&args, "--compare-visio-jumps") {
        // (JD21v) line jumps laid here against a file Visio laid anew
        return jumps::compare(&file);
    }
    if let Some(dir) = arg(&args, "--write-jd21-groups") {
        // (JD21) groups, boundary members, Shape ändern for the Visio counter-check
        return groups::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-jd21-route") {
        // (JD21) the routing styles drawing for the Visio counter-check, a PNG per page
        return route::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-jd21-ops") {
        // (JD21) the shape operations for the Visio counter-check
        return shapeops::write(&dir);
    }
    if let Some(dir) = arg(&args, "--write-probe") {
        return roundtrip::write_probe(&dir, arg(&args, "--corpus").as_deref());
    }
    let check = args.iter().any(|a| a == "--check");
    let corpus_dir = arg(&args, "--corpus");
    if !check && corpus_dir.is_none() {
        out("Aufruf: diagramiotest --check [--only NAME] [--corpus DIR] | --corpus DIR | --write-samples DIR | --write-golden DIR | --write-drawings DIR | --render-corpus DIR [--width PX] | --write-stencils DIR | --write-stencil-previews DIR | --write-gegenprobe DIR\n");
        return 2;
    }
    let mut h = H { ok: 0, failed: 0, only: arg(&args, "--only") };
    let t0 = plat::time_us();
    if check {
        if h.on("samples") {
            samples::run(&mut h);
        }
        hostile::run(&mut h);
        if h.on("render") {
            render::run(&mut h);
        }
        if h.on("export") {
            export::run(&mut h);
        }
        if h.on("editview") {
            edit_view::run(&mut h);
        }
        if h.on("edit") {
            gegenprobe::check(&mut h);
        }
        if h.on("designs") {
            designs::check(&mut h);
        }
        roundtrip::run(&mut h, corpus_dir.as_deref());
        if h.on("stencils") {
            stencils::run(&mut h);
            stencil_structs::run(&mut h);
            stencil_shapes::run(&mut h);
        }
        if h.on("structures") {
            structures::check(&mut h);
        }
        // (JD13) the network and rack stencils
        if h.on("stencils/network") {
            stencil_network::run(&mut h);
        }
        // (JD26) the general and business-graphics stencils
        if h.on("stencils/business") {
            stencil_business::run(&mut h);
        }
        if h.on("templates") {
            templates::run(&mut h);
        }
        // (JD16) the XML formats of 2003-2010, the macro parts, the document properties
        if h.on("vdx") {
            vdx::run(&mut h);
        }
        // (JD18) external data (DF-12)
        if h.on("data") {
            data::run(&mut h);
        }
        // (JD21) the shape operations
        if h.on("shapeops") {
            shapeops::check(&mut h);
        }
        // (JD21) groups, boundary members, Shape ändern
        if h.on("groups") {
            groups::check(&mut h);
        }
        // (JD19) data graphics
        if h.on("datagraphics") {
            datagraphics::run(&mut h);
        }
        // (JD19) shape reports (DA-09)
        if h.on("reports") {
            reports::run(&mut h);
        }
        // (JD17) the binary stencils of 2003-2010 (subgroups vss/cfb, vss/stream ...)
        vss::run(&mut h);
        vss_sample::run(&mut h);
        // (JD21) line jumps
        if h.on("jumps") {
            jumps::check(&mut h);
        }
        // (JD21) routing styles, permeable shapes, a way by hand
        if h.on("route") {
            route::check(&mut h, corpus_dir.clone());
        }
        // (JD21 glue) glue of control points, to vertices, handles, guides; new connection points
        if h.on("pointglue") {
            pointglue::check(&mut h);
        }
    }
    if h.on("corpus") {
        corpus::run(&mut h, corpus_dir.clone());
    }
    if h.on("render") || h.on("corpus") {
        render::corpus(&mut h, corpus_dir.clone());
    }
    if h.on("formulas") {
        formulas::run(&mut h, corpus_dir, arg(&args, "--show").and_then(|n| n.parse().ok()).unwrap_or(0));
    }
    out(&format!("diagramiotest: {} ok, {} failed ({} ms)\n", h.ok, h.failed, plat::time_us().wrapping_sub(t0) / 1000));
    if h.failed == 0 { 0 } else { 1 }
}

rt::main!(run());
