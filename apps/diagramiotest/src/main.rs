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
//   diagramiotest --render-corpus DIR [--width PX]   (JD4) the local files' pages as PNG and SVG
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
//   formulas  the formula oracle (JD6): every formula cell of the local files recomputed and
//             compared with the stored value (--show N prints N misses, locally)
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

mod corpus;
mod drawings;
mod formulas;
mod hostile;
mod pkg;
mod render;
mod samples;
mod svgdump;
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
    let check = args.iter().any(|a| a == "--check");
    let corpus_dir = arg(&args, "--corpus");
    if !check && corpus_dir.is_none() {
        out("Aufruf: diagramiotest --check [--only NAME] [--corpus DIR] | --corpus DIR | --write-samples DIR | --write-golden DIR | --write-drawings DIR | --render-corpus DIR [--width PX]\n");
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
