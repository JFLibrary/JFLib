// unit -- svgtest --test unit: self-checks of img_svg through its public API (parse, render,
// the codec), pixel by pixel on small renders: sizes and units, colours, path data, transforms,
// viewBox / preserveAspectRatio, strokes and dashes, gradients, opacity and layers, clip paths,
// CSS, use / symbol, display / visibility, nested viewports, embedded images, the img_core
// format table; then malformed input (truncations, mutations of every fixture, deep nesting,
// entity and use explosions, cycles, absurd numbers) which must neither panic nor hang.
// Prints FAIL lines and a summary; the exit code is the number of failures.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Surface;
use img_svg::{Opts, Svg};

use crate::FIXTURES;

struct T {
    group: &'static str,
    ok: u32,
    failed: u32,
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

impl T {
    fn check(&mut self, cond: bool, name: &str) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            out(&format!("FAIL {}: {}\n", self.group, name));
        }
    }
    fn eq<A: PartialEq + core::fmt::Debug>(&mut self, got: A, want: A, name: &str) {
        let ok = got == want;
        self.check(ok, &if ok { String::from(name) } else { format!("{}: got {:?}, want {:?}", name, got, want) });
    }
    // a premultiplied pixel near (a, r, g, b) (each channel within tol)
    fn px(&mut self, sf: &Surface, x: i32, y: i32, want: u32, tol: u32, name: &str) {
        let got = px(sf, x, y);
        let ok = (0..4).all(|k| ((got >> (k * 8)) & 255).abs_diff((want >> (k * 8)) & 255) <= tol);
        self.check(ok, &if ok { String::from(name) } else { format!("{} at ({}, {}): got {:08X}, want {:08X}", name, x, y, got, want) });
    }
    fn alpha(&mut self, sf: &Surface, x: i32, y: i32, want: u32, tol: u32, name: &str) {
        let a = px(sf, x, y) >> 24;
        let ok = a.abs_diff(want) <= tol;
        self.check(ok, &if ok { String::from(name) } else { format!("{} at ({}, {}): alpha {}, want {}", name, x, y, a, want) });
    }
}

fn px(sf: &Surface, x: i32, y: i32) -> u32 {
    if x < 0 || y < 0 || x >= sf.w || y >= sf.h {
        return 0xDEAD_BEEF;
    }
    let o = y as usize * sf.stride as usize + x as usize * 4;
    u32::from_le_bytes([sf.px[o], sf.px[o + 1], sf.px[o + 2], sf.px[o + 3]])
}

/// render an SVG source at w x h (the document's viewport)
fn r(src: &str, w: i32, h: i32) -> Surface {
    match Svg::parse(src.as_bytes()) {
        Ok(s) => s.rasterize(w, h, &mut Opts::default()).unwrap_or_default(),
        Err(_) => Surface::default(),
    }
}

/// a 100 x 100 document around some content
fn doc(body: &str) -> String {
    format!("<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100' viewBox='0 0 100 100'>{}</svg>", body)
}

fn r100(body: &str) -> Surface {
    r(&doc(body), 100, 100)
}

fn same(a: &Surface, b: &Surface) -> bool {
    a.w == b.w && a.h == b.h && a.px == b.px
}

// a deterministic generator for the mutations
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

const SYMS: &[u8] = b"<>/=\"' &;#-.0123456789eE:()[]{}%xX";
const RED: u32 = 0xFFFF_0000;
const BLUE: u32 = 0xFF00_00FF;
const NONE: u32 = 0;

fn parse_and_sizes(t: &mut T) {
    t.group = "parse";
    let s = Svg::parse(b"<svg width='120' height='80'/>").unwrap();
    t.eq(s.size(), (120.0, 80.0), "width / height");
    let s = Svg::parse(b"<svg viewBox='0 0 30 20'/>").unwrap();
    t.eq(s.size(), (30.0, 20.0), "size from the viewBox");
    let s = Svg::parse(b"<svg width='60' viewBox='0 0 30 20'/>").unwrap();
    t.eq(s.size(), (60.0, 40.0), "height from the aspect ratio");
    let s = Svg::parse(b"<svg/>").unwrap();
    t.eq(s.size(), (300.0, 150.0), "default 300 x 150");
    let s = Svg::parse(b"<svg width='1in' height='72pt'/>").unwrap();
    t.eq(s.size(), (96.0, 96.0), "in / pt");
    let s = Svg::parse(b"<svg width='100%' height='50%' viewBox='0 0 10 5'/>").unwrap();
    t.eq(s.size(), (10.0, 5.0), "percent sizes -> viewBox");
    let s = Svg::parse(b"<svg width='-5' height='abc' viewBox='0 0 0 10'/>").unwrap();
    t.eq(s.size(), (300.0, 150.0), "invalid sizes and viewBox");
    let s = Svg::parse(b"<svg><title>  Ein\n  Titel </title></svg>").unwrap();
    t.eq(s.title(), "Ein Titel", "title (blanks collapsed)");
    t.check(Svg::parse(b"").is_err(), "empty input");
    t.check(Svg::parse(b"<html><svg/></html>").is_err(), "root is not svg");
    t.check(Svg::parse(b"hello world").is_err(), "no element");
    t.check(Svg::parse(&[0xFF, 0xFE, b'<', 0]).is_err(), "UTF-16");
    let s = Svg::parse(b"<svg><g><rect width='5' height='5'/>").unwrap();
    t.check(s.truncated(), "unclosed elements: truncated");
    t.check(!Svg::parse(b"<svg><g/></svg>").unwrap().truncated(), "complete: not truncated");
    t.check(Svg::parse(b"\xEF\xBB\xBF<?xml version='1.0'?><!-- c --><!DOCTYPE svg [<!ENTITY a 'b'>]><svg/>").is_ok(), "BOM, declaration, comment, DOCTYPE");
    let s = Svg::parse(b"<svg:svg xmlns:svg='http://www.w3.org/2000/svg' width='7' height='7'/>").unwrap();
    t.eq(s.size(), (7.0, 7.0), "svg: prefix");
    // sniffing
    t.check(img_svg::sniff(b"  <svg>"), "sniff: blanks");
    t.check(img_svg::sniff(b"<?xml version='1.0'?>\n<!-- x -->\n<!DOCTYPE svg [ <!ENTITY a '>'> ]>\n<svg width='1'>"), "sniff: prolog");
    t.check(!img_svg::sniff(b"<html><svg>"), "sniff: html");
    t.check(!img_svg::sniff(b"<svgx>"), "sniff: another name");
    t.check(!img_svg::sniff(b"\x89PNG\r\n\x1a\n"), "sniff: PNG");
    // every fixture parses
    for (name, data) in FIXTURES {
        match Svg::parse(data) {
            Ok(s) => {
                t.check(!s.truncated(), &format!("{} complete", name));
                t.check(s.element_count() > 2, &format!("{} elements", name));
            }
            Err(e) => t.check(false, &format!("{} parses ({})", name, e)),
        }
    }
    let ed = Svg::parse(FIXTURES.iter().find(|f| f.0 == "editor").unwrap().1).unwrap();
    let (w, _) = ed.size();
    t.check((w - 200.0).abs() < 0.1, &format!("mm units: 52.917mm = {:.2} px", w));
    t.eq(ed.title(), "Editor-Datei", "entities, svg: prefix");
}

fn colors(t: &mut T) {
    t.group = "colors";
    let cases: [(&str, u32); 14] = [
        ("#f00", RED),
        ("#ff0000", RED),
        ("#FF0000", RED),
        ("red", RED),
        ("RED", RED),
        ("rgb(255, 0, 0)", RED),
        ("rgb(100%, 0%, 0%)", RED),
        ("rgb(255 0 0)", RED),
        ("hsl(240, 100%, 50%)", BLUE),
        ("rebeccapurple", 0xFF66_3399),
        ("#0000ff80", 0x8000_0080),
        ("rgba(0, 0, 255, 0.5)", 0x8000_0080),
        ("#00f8", 0x8800_0088),
        ("transparent", NONE),
    ];
    for (c, want) in cases {
        let sf = r100(&format!("<rect width='100' height='100' fill='{}'/>", c));
        t.px(&sf, 50, 50, want, 1, c);
    }
    // invalid colours: the declaration is ignored (black stays)
    let sf = r100("<rect width='100' height='100' fill='#12'/>");
    t.px(&sf, 50, 50, 0xFF00_0000, 0, "invalid -> inherited black");
    let sf = r100("<g fill='blue'><rect width='100' height='100' fill='bogus'/></g>");
    t.px(&sf, 50, 50, BLUE, 0, "invalid -> inherited blue");
    // currentColor: the color property, else the renderer's
    let sf = r100("<rect width='100' height='100' fill='currentColor' color='#00ff00'/>");
    t.px(&sf, 50, 50, 0xFF00_FF00, 0, "currentColor from color");
    let svg = Svg::parse(doc("<rect width='100' height='100' fill='currentColor'/>").as_bytes()).unwrap();
    let sf = svg.rasterize(10, 10, &mut Opts { color: 0xFF12_3456, text: None }).unwrap();
    t.px(&sf, 5, 5, 0xFF12_3456, 0, "currentColor from Opts");
    let sf = r100("<g color='#00ff00'><rect width='100' height='100' fill='currentColor' color='inherit'/></g>");
    t.px(&sf, 5, 5, 0xFF00_FF00, 0, "color inherit");
    let sf = r100("<rect width='100' height='100' fill='red' fill-opacity='0.5'/>");
    t.px(&sf, 50, 50, 0x8080_0000, 1, "fill-opacity");
    let sf = r100("<rect width='100' height='100' fill='red' fill-opacity='50%'/>");
    t.px(&sf, 50, 50, 0x8080_0000, 1, "fill-opacity percent");
    let sf = r100("<rect width='100' height='100'/>");
    t.px(&sf, 50, 50, 0xFF00_0000, 0, "default fill black");
    let sf = r100("<rect width='100' height='100' fill='none' stroke='red'/>");
    t.px(&sf, 50, 50, NONE, 0, "fill none");
}

fn paths(t: &mut T) {
    t.group = "paths";
    let sf = r100("<path d='M10 10 h80 v80 h-80 z' fill='red'/>");
    t.px(&sf, 50, 50, RED, 0, "relative square inside");
    t.px(&sf, 5, 50, NONE, 0, "relative square outside");
    let sf = r100("<path d='M50 10 A40 40 0 1 1 49.99 10 Z' fill='red'/>");
    t.px(&sf, 50, 50, RED, 0, "arc circle centre");
    t.px(&sf, 50, 85, RED, 0, "arc circle bottom");
    t.px(&sf, 15, 15, NONE, 0, "arc circle corner");
    let sf = r100("<path d='M10 50 a40 40 0 0 1 80 0 z' fill='red'/>");
    t.px(&sf, 50, 30, RED, 0, "half circle top (sweep 1)");
    t.px(&sf, 50, 70, NONE, 0, "half circle: no bottom");
    let sf = r100("<path d='M10 50 a40 40 0 0 0 80 0 z' fill='red'/>");
    t.px(&sf, 50, 70, RED, 0, "half circle bottom (sweep 0)");
    // radii too small are scaled up
    let sf = r100("<path d='M10 50 A1 1 0 0 1 90 50 z' fill='red'/>");
    t.px(&sf, 50, 30, RED, 0, "arc radii scaled up");
    // implicit lineto after moveto, repeated commands
    let sf = r100("<path d='M10 10 90 10 90 90 10 90z' fill='red'/>");
    t.px(&sf, 50, 50, RED, 0, "implicit lineto");
    let sf = r100("<path d='M0 0L100 0 100 50 0 50z M0 60h100v40H0z' fill='red'/>");
    t.px(&sf, 50, 25, RED, 0, "two subpaths: first");
    t.px(&sf, 50, 55, NONE, 0, "two subpaths: gap");
    t.px(&sf, 50, 80, RED, 0, "two subpaths: second");
    // curves: C S Q T
    let sf = r100("<path d='M10 90 C10 10 90 10 90 90 Z' fill='red'/>");
    t.px(&sf, 50, 60, RED, 0, "cubic");
    t.px(&sf, 50, 15, NONE, 0, "cubic: above the curve");
    let sf = r100("<path d='M10 90 Q50 -10 90 90 Z' fill='red'/>");
    t.px(&sf, 50, 60, RED, 0, "quadratic");
    let sf = r100("<path d='M0 50 Q25 0 50 50 T100 50 V100 H0Z' fill='red'/>");
    t.px(&sf, 75, 90, RED, 0, "smooth quadratic: below the reflected hump");
    t.px(&sf, 75, 60, NONE, 0, "smooth quadratic: the reflected hump dips");
    t.px(&sf, 25, 35, RED, 0, "smooth quadratic: inside the first hump");
    // an error ends the path: what came before is drawn
    let sf = r100("<path d='M0 0 H50 V50 H0 Z M60 60 L 70 # 80 80' fill='red'/>");
    t.px(&sf, 25, 25, RED, 0, "error: prefix drawn");
    let sf = r100("<path d='L10 10 50 50' fill='red' stroke='red'/>");
    t.px(&sf, 30, 30, NONE, 0, "no moveto: nothing");
    // fill-rule
    let star = "M50 5 L79 95 L2 39 L98 39 L21 95 Z";
    let sf = r100(&format!("<path d='{}' fill='red'/>", star));
    t.px(&sf, 50, 55, RED, 0, "nonzero: centre filled");
    let sf = r100(&format!("<path d='{}' fill='red' fill-rule='evenodd'/>", star));
    t.px(&sf, 50, 55, NONE, 0, "evenodd: centre open");
    // polyline / polygon / line, odd point lists
    let sf = r100("<polygon points='0,0 100,0 100,100 0,100 50' fill='red'/>");
    t.px(&sf, 50, 50, RED, 0, "polygon (odd count)");
    let sf = r100("<polyline points='0 0, 100 0 100 100' fill='red'/>");
    t.px(&sf, 90, 20, RED, 0, "polyline filled");
    t.px(&sf, 10, 80, NONE, 0, "polyline: the open side");
    let sf = r100("<line x1='0' y1='50' x2='100' y2='50' stroke='red' stroke-width='10' fill='blue'/>");
    t.px(&sf, 50, 50, RED, 0, "line stroked");
    // shapes
    let sf = r100("<rect x='10' y='10' width='80' height='80' rx='20' fill='red'/>");
    t.px(&sf, 12, 12, NONE, 0, "rect rx corner");
    t.px(&sf, 50, 12, RED, 0, "rect rx edge");
    let sf = r100("<rect x='10' y='10' width='80' height='80' ry='40' fill='red'/>");
    t.px(&sf, 15, 50, RED, 0, "rect ry only = rx");
    t.px(&sf, 20, 20, NONE, 0, "rect ry only: rounded");
    let sf = r100("<rect width='-10' height='50' fill='red'/><rect width='0' height='50' fill='red'/>");
    t.px(&sf, 5, 5, NONE, 0, "negative / zero width: nothing");
    let sf = r100("<circle cx='50' cy='50' r='40' fill='red'/><circle r='-5' fill='red'/>");
    t.px(&sf, 50, 50, RED, 0, "circle");
    t.px(&sf, 12, 12, NONE, 0, "circle corner");
    let sf = r100("<ellipse cx='50' cy='50' rx='45' ry='20' fill='red'/>");
    t.px(&sf, 90, 50, RED, 0, "ellipse wide");
    t.px(&sf, 50, 25, NONE, 0, "ellipse low");
    let sf = r100("<rect x='10%' y='10%' width='80%' height='80%' fill='red'/>");
    t.px(&sf, 50, 50, RED, 0, "percent lengths");
    t.px(&sf, 5, 5, NONE, 0, "percent lengths outside");
}

fn transforms(t: &mut T) {
    t.group = "transforms";
    let sf = r100("<rect width='20' height='20' fill='red' transform='translate(50 50)'/>");
    t.px(&sf, 60, 60, RED, 0, "translate");
    t.px(&sf, 10, 10, NONE, 0, "translate: origin empty");
    let sf = r100("<rect width='20' height='20' fill='red' transform='scale(4)'/>");
    t.px(&sf, 70, 70, RED, 0, "scale");
    let sf = r100("<rect width='40' height='10' fill='red' transform='rotate(90 50 50)'/>");
    t.px(&sf, 50, 50, NONE, 0, "rotate about a point: the old place is empty");
    let sf = r100("<rect x='10' y='45' width='80' height='10' fill='red' transform='rotate(90 50 50)'/>");
    t.px(&sf, 50, 20, RED, 0, "rotate 90: vertical bar");
    t.px(&sf, 20, 50, NONE, 0, "rotate 90: no horizontal bar");
    let a = r100("<g transform='translate(10,20) scale(2) rotate(30)'><rect width='20' height='15' fill='red'/></g>");
    let b = r100("<g transform='translate(10,20)'><g transform='scale(2)'><g transform='rotate(30)'><rect width='20' height='15' fill='red'/></g></g></g>");
    t.check(same(&a, &b), "a list = nested groups");
    let c = r100("<g transform='matrix(1.7320508 1 -1 1.7320508 10 20)'><rect width='20' height='15' fill='red'/></g>");
    let diff = a.px.iter().zip(c.px.iter()).filter(|(x, y)| x.abs_diff(**y) > 2).count();
    t.check(diff == 0, &format!("matrix() = translate scale rotate ({} bytes differ)", diff));
    let sf = r100("<rect width='20' height='20' fill='red' transform='skewX(45)' y='40' x='10'/>");
    t.px(&sf, 70, 50, RED, 0, "skewX");
    t.px(&sf, 25, 50, NONE, 0, "skewX: moved right");
    let sf = r100("<rect width='20' height='20' fill='red' transform='translate(50,50) bogus(1)'/>");
    t.px(&sf, 10, 10, RED, 0, "invalid transform: none");
    let sf = r100("<rect width='100' height='100' fill='red' transform='scale(0)'/>");
    t.px(&sf, 0, 0, NONE, 0, "scale(0): nothing");
}

fn viewbox(t: &mut T) {
    t.group = "viewBox";
    // a 10 x 10 viewBox in a 100 x 50 viewport
    let src = |par: &str| format!("<svg width='100' height='50' viewBox='0 0 10 10' preserveAspectRatio='{}'><rect width='10' height='10' fill='red'/></svg>", par);
    let sf = r(&src("xMidYMid meet"), 100, 50);
    t.px(&sf, 50, 25, RED, 0, "meet: centre");
    t.px(&sf, 10, 25, NONE, 0, "meet: left free");
    t.px(&sf, 70, 25, RED, 0, "meet: right part");
    let sf = r(&src("xMinYMin meet"), 100, 50);
    t.px(&sf, 10, 25, RED, 0, "xMin: left");
    t.px(&sf, 80, 25, NONE, 0, "xMin: right free");
    let sf = r(&src("xMaxYMax meet"), 100, 50);
    t.px(&sf, 90, 25, RED, 0, "xMax: right");
    let sf = r(&src("none"), 100, 50);
    t.px(&sf, 2, 2, RED, 0, "none: stretched");
    t.px(&sf, 98, 48, RED, 0, "none: stretched corner");
    // slice: the viewBox covers everything (the part outside is cut)
    let sf = r("<svg width='100' height='50' viewBox='0 0 10 10' preserveAspectRatio='xMidYMid slice'><rect width='10' height='5' fill='red'/></svg>", 100, 50);
    t.px(&sf, 50, 10, RED, 0, "slice: top half of the viewBox visible");
    t.px(&sf, 50, 40, NONE, 0, "slice: the lower half below");
    // rasterize at another size keeps the aspect ratio of the viewBox
    let svg = Svg::parse(b"<svg width='10' height='10' viewBox='0 0 10 10'><circle cx='5' cy='5' r='5' fill='red'/></svg>").unwrap();
    let sf = svg.rasterize(200, 100, &mut Opts::default()).unwrap();
    t.px(&sf, 100, 50, RED, 0, "rasterize larger: centre");
    t.px(&sf, 20, 50, NONE, 0, "rasterize larger: meet keeps the circle round");
    // content outside the root viewport is clipped
    let svg = Svg::parse(b"<svg width='10' height='10'><rect x='-50' y='-50' width='200' height='200' fill='red'/></svg>").unwrap();
    let mut big = Surface::new(30, 30, gfx::FMT_PARGB);
    svg.render(&mut big, &gfx::xform::Matrix { c: 10.0, f: 10.0, ..gfx::xform::Matrix::IDENTITY }, None, &mut Opts::default());
    t.px(&big, 15, 15, RED, 0, "render: inside the viewport");
    t.px(&big, 5, 5, NONE, 0, "render: clipped outside the viewport");
    // a nested svg clips to its viewport; overflow visible does not
    let sf = r100("<svg x='20' y='20' width='20' height='20'><rect width='100' height='100' fill='red'/></svg>");
    t.px(&sf, 30, 30, RED, 0, "nested svg inside");
    t.px(&sf, 60, 60, NONE, 0, "nested svg clipped");
    let sf = r100("<svg x='20' y='20' width='20' height='20' overflow='visible'><rect width='100' height='100' fill='red'/></svg>");
    t.px(&sf, 60, 60, RED, 0, "nested svg overflow visible");
    let sf = r100("<svg x='0' y='0' width='50' height='50' viewBox='0 0 10 10'><rect width='5' height='5' fill='red'/></svg>");
    t.px(&sf, 20, 20, RED, 0, "nested viewBox scales");
    t.px(&sf, 30, 30, NONE, 0, "nested viewBox scale bound");
}

fn strokes(t: &mut T) {
    t.group = "strokes";
    let sf = r100("<line x1='20' y1='50' x2='80' y2='50' stroke='red' stroke-width='10'/>");
    t.px(&sf, 50, 47, RED, 0, "width 10: inside");
    t.px(&sf, 50, 40, NONE, 0, "width 10: outside");
    t.px(&sf, 17, 50, NONE, 0, "butt cap: nothing before the start");
    let sf = r100("<line x1='20' y1='50' x2='80' y2='50' stroke='red' stroke-width='10' stroke-linecap='square'/>");
    t.px(&sf, 17, 50, RED, 0, "square cap");
    let sf = r100("<line x1='20' y1='50' x2='80' y2='50' stroke='red' stroke-width='10' stroke-linecap='round'/>");
    t.px(&sf, 17, 50, RED, 0, "round cap: on the axis");
    t.px(&sf, 15, 45, NONE, 0, "round cap: not in the corner");
    let sf = r100("<line x1='0' y1='50' x2='100' y2='50' stroke='red' stroke-width='10' stroke-dasharray='10 10'/>");
    t.px(&sf, 5, 50, RED, 0, "dash on");
    t.px(&sf, 15, 50, NONE, 0, "dash off");
    t.px(&sf, 25, 50, RED, 0, "dash on again");
    let sf = r100("<line x1='0' y1='50' x2='100' y2='50' stroke='red' stroke-width='10' stroke-dasharray='10 10' stroke-dashoffset='10'/>");
    t.px(&sf, 5, 50, NONE, 0, "dashoffset shifts");
    t.px(&sf, 15, 50, RED, 0, "dashoffset shifts on");
    let sf = r100("<line x1='0' y1='50' x2='100' y2='50' stroke='red' stroke-width='10' stroke-dasharray='5'/>");
    t.px(&sf, 2, 50, RED, 0, "odd dasharray repeated: on");
    t.px(&sf, 7, 50, NONE, 0, "odd dasharray repeated: off");
    let sf = r100("<line x1='0' y1='50' x2='100' y2='50' stroke='red' stroke-width='10' stroke-dasharray='5 -1'/>");
    t.px(&sf, 7, 50, RED, 0, "negative dash: solid");
    let sf = r100("<line x1='0' y1='50' x2='100' y2='50' stroke='red' stroke-width='10' stroke-dasharray='0 0'/>");
    t.px(&sf, 7, 50, RED, 0, "zero dashes: solid");
    // joins: a right angle; the miter fills the outer corner
    let pl = |join: &str| r100(&format!("<polyline points='20,80 20,20 80,20' fill='none' stroke='red' stroke-width='20' stroke-linejoin='{}'/>", join));
    let sf = pl("miter");
    t.px(&sf, 12, 12, RED, 0, "miter corner");
    let sf = pl("bevel");
    t.px(&sf, 11, 11, NONE, 0, "bevel corner cut");
    let sf = pl("round");
    t.px(&sf, 11, 11, NONE, 0, "round corner cut");
    t.px(&sf, 15, 15, RED, 0, "round corner arc");
    // miter limit: a sharp angle beyond the limit is bevelled
    let sharp = |lim: &str| r100(&format!("<polyline points='10,90 50,10 90,90' fill='none' stroke='red' stroke-width='10' stroke-miterlimit='{}'/>", lim));
    let sf = sharp("10");
    t.px(&sf, 50, 2, RED, 0, "within the miter limit: pointed");
    let sf = sharp("1");
    t.px(&sf, 50, 2, NONE, 0, "beyond the miter limit: bevelled");
    // closed subpaths have no caps; stroke-opacity; width in percent and em
    let sf = r100("<rect x='20' y='20' width='60' height='60' fill='none' stroke='red' stroke-width='10' stroke-opacity='0.5'/>");
    t.px(&sf, 20, 50, 0x8080_0000, 1, "stroke-opacity");
    t.px(&sf, 50, 50, NONE, 0, "stroke only");
    let sf = r100("<line x1='0' y1='50' x2='100' y2='50' stroke='red' stroke-width='1em' font-size='20'/>");
    t.px(&sf, 50, 58, RED, 0, "stroke-width in em");
    let sf = r100("<g transform='scale(2)'><line x1='0' y1='25' x2='50' y2='25' stroke='red' stroke-width='5'/></g>");
    t.px(&sf, 50, 46, RED, 0, "stroke scales with the transform");
    t.px(&sf, 50, 43, NONE, 0, "stroke scaled bound");
    let sf = r100("<line x1='0' y1='50' x2='100' y2='50' stroke='red' stroke-width='0'/>");
    t.px(&sf, 50, 50, NONE, 0, "width 0: nothing");
    let sf = r100("<path d='M50 50 z' stroke='red' stroke-width='20' stroke-linecap='round'/>");
    t.px(&sf, 50, 50, RED, 0, "zero length + round cap: a dot");
    let sf = r100("<path d='M20 20 M50 50 L80 50 M90 90' stroke='red' stroke-width='10' stroke-linecap='round' fill='none'/>");
    t.px(&sf, 20, 20, NONE, 0, "a lone moveto draws no dot");
    t.px(&sf, 90, 90, NONE, 0, "a trailing moveto draws no dot");
    t.px(&sf, 65, 50, RED, 0, "the segment between");
    let sf = r100("<path d='M50 50 L50 50' stroke='red' stroke-width='20' stroke-linecap='square'/>");
    t.px(&sf, 55, 55, RED, 0, "zero length + square cap: a square");
}

fn gradients(t: &mut T) {
    t.group = "gradients";
    let defs = "<defs>\
        <linearGradient id='lg'><stop offset='0' stop-color='#ff0000'/><stop offset='1' stop-color='#0000ff'/></linearGradient>\
        <linearGradient id='refl' x1='0' x2='0.5' spreadMethod='reflect'><stop offset='0' stop-color='#ff0000'/><stop offset='1' stop-color='#0000ff'/></linearGradient>\
        <linearGradient id='rep' x1='0' x2='0.5' spreadMethod='repeat'><stop offset='0' stop-color='#ff0000'/><stop offset='1' stop-color='#0000ff'/></linearGradient>\
        <linearGradient id='user' gradientUnits='userSpaceOnUse' x1='0' x2='50'><stop offset='0' stop-color='#ff0000'/><stop offset='1' stop-color='#0000ff'/></linearGradient>\
        <linearGradient id='child' href='#lg' x1='1' x2='0'/>\
        <linearGradient id='rot' href='#lg' gradientTransform='rotate(90 0.5 0.5)'/>\
        <linearGradient id='one'><stop offset='0.3' stop-color='#00ff00'/></linearGradient>\
        <linearGradient id='empty'/>\
        <linearGradient id='op'><stop offset='0' stop-color='#ff0000' stop-opacity='0.5'/><stop offset='1' stop-color='#ff0000' stop-opacity='0.5'/></linearGradient>\
        <radialGradient id='rg'><stop offset='0' stop-color='#ffffff'/><stop offset='1' stop-color='#000000'/></radialGradient>\
        <radialGradient id='foc' fx='0.1' fy='0.5'><stop offset='0' stop-color='#ffffff'/><stop offset='1' stop-color='#000000'/></radialGradient>\
        <pattern id='pat' width='10' height='10'><rect width='5' height='5'/></pattern>\
        </defs>";
    let g = |fill: &str| r100(&format!("{}<rect width='100' height='100' fill='{}'/>", defs, fill));
    let sf = g("url(#lg)");
    t.px(&sf, 0, 50, 0xFFFE_0001, 2, "linear: red at the start");
    t.px(&sf, 99, 50, 0xFF01_00FE, 2, "linear: blue at the end");
    t.px(&sf, 50, 50, 0xFF80_007F, 3, "linear: the middle");
    t.check(px(&sf, 50, 0) == px(&sf, 50, 99), "linear: constant across");
    let sf = g("url(#refl)");
    t.px(&sf, 49, 50, 0xFF00_00FF, 5, "reflect: blue at 0.5");
    t.px(&sf, 99, 50, 0xFFFF_0000, 5, "reflect: red again at 1");
    let sf = g("url(#rep)");
    t.px(&sf, 48, 50, 0xFF05_00FA, 8, "repeat: blue before 0.5");
    t.px(&sf, 51, 50, 0xFFFA_0005, 8, "repeat: red after 0.5");
    let sf = g("url(#user)");
    t.px(&sf, 49, 50, 0xFF00_00FF, 6, "userSpaceOnUse: blue at x 50");
    t.px(&sf, 90, 50, 0xFF00_00FF, 0, "userSpaceOnUse: padded");
    let sf = g("url(#child)");
    t.px(&sf, 0, 50, 0xFF00_00FF, 3, "href: stops inherited, x1 / x2 own");
    let sf = g("url(#rot)");
    t.px(&sf, 50, 0, 0xFFFF_0000, 3, "gradientTransform rotate: red at the top");
    t.px(&sf, 50, 99, 0xFF00_00FF, 3, "gradientTransform rotate: blue at the bottom");
    let sf = g("url(#one)");
    t.px(&sf, 10, 10, 0xFF00_FF00, 0, "one stop: solid");
    let sf = g("url(#empty) red");
    t.px(&sf, 10, 10, NONE, 0, "no stops: nothing (not the fallback)");
    let sf = g("url(#missing) red");
    t.px(&sf, 10, 10, RED, 0, "missing reference: the fallback");
    let sf = g("url(#missing)");
    t.px(&sf, 10, 10, NONE, 0, "missing reference without fallback: nothing");
    let sf = g("url(#pat) #0000ff");
    t.px(&sf, 10, 10, BLUE, 0, "pattern: the fallback");
    let sf = g("url(#op)");
    t.px(&sf, 50, 50, 0x8080_0000, 1, "stop-opacity");
    let sf = g("url(#rg)");
    t.px(&sf, 50, 50, 0xFFFF_FFFF, 6, "radial: white in the centre");
    t.px(&sf, 99, 50, 0xFF00_0000, 6, "radial: black at the edge");
    t.px(&sf, 2, 2, 0xFF00_0000, 0, "radial: padded beyond");
    let sf = g("url(#foc)");
    t.check(px(&sf, 12, 50) & 255 > 200, "focal: bright near the focus");
    t.check(px(&sf, 80, 50) & 255 < 160, "focal: darker on the far side");
    // objectBoundingBox follows the shape
    let sf = r100(&format!("{}<rect x='50' width='50' height='100' fill='url(#lg)'/>", defs));
    t.px(&sf, 50, 50, 0xFFFE_0001, 2, "bbox: starts at the shape");
    // a zero-size bounding box: not painted
    let sf = r100(&format!("{}<line x1='10' y1='50' x2='90' y2='50' stroke='url(#lg)' stroke-width='10'/>", defs));
    t.px(&sf, 50, 50, NONE, 0, "bbox of zero height: no paint");
    // a gradient href cycle ends
    let sf = r100("<defs><linearGradient id='a' href='#b'/><linearGradient id='b' href='#a'/></defs><rect width='10' height='10' fill='url(#a) red'/>");
    t.px(&sf, 5, 5, NONE, 0, "href cycle: no stops, nothing");
}

fn opacity(t: &mut T) {
    t.group = "opacity";
    let sf = r100("<g opacity='0.5'><rect width='60' height='60' fill='red'/><rect x='40' y='40' width='60' height='60' fill='red'/></g>");
    t.alpha(&sf, 20, 20, 128, 1, "group: single");
    t.alpha(&sf, 50, 50, 128, 1, "group: overlap not darker");
    let sf = r100("<rect width='60' height='60' fill='red' opacity='0.5'/><rect x='40' y='40' width='60' height='60' fill='red' opacity='0.5'/>");
    t.alpha(&sf, 50, 50, 191, 2, "elements: overlap adds");
    let sf = r100("<g opacity='0.5'><g opacity='0.5'><rect width='100' height='100' fill='red'/></g></g>");
    t.alpha(&sf, 50, 50, 64, 2, "nested multiply");
    let sf = r100("<rect width='100' height='100' fill='red' stroke='blue' stroke-width='40' opacity='0.5'/>");
    t.px(&sf, 5, 50, 0x8000_0080, 2, "fill + stroke in a layer: the stroke covers the fill");
    // deeper than the layer limit: the opacity still applies
    let mut s = String::new();
    for _ in 0..12 {
        s.push_str("<g opacity='0.9'>");
    }
    s.push_str("<rect width='100' height='100' fill='red'/>");
    for _ in 0..12 {
        s.push_str("</g>");
    }
    let sf = r100(&s);
    t.alpha(&sf, 50, 50, 72, 4, "12 nested groups (0.9^12)");
    let sf = r100("<rect width='100' height='100' fill='red' opacity='0'/>");
    t.alpha(&sf, 50, 50, 0, 0, "opacity 0");
    let sf = r100("<rect width='100' height='100' fill='red' opacity='2'/>");
    t.alpha(&sf, 50, 50, 255, 0, "opacity clamped");
}

fn clipping(t: &mut T) {
    t.group = "clip";
    let defs = "<defs>\
        <clipPath id='c'><circle cx='50' cy='50' r='30'/></clipPath>\
        <clipPath id='b' clipPathUnits='objectBoundingBox'><rect x='0.5' width='0.5' height='1'/></clipPath>\
        <clipPath id='eo'><path d='M10 10 H90 V90 H10 Z M30 30 V70 H70 V30 Z' clip-rule='evenodd'/></clipPath>\
        <clipPath id='nest' clip-path='url(#c)'><rect width='50' height='100'/></clipPath>\
        <clipPath id='t'><rect width='50' height='50' transform='translate(50 50)'/></clipPath>\
        <clipPath id='u'><use href='#shape'/></clipPath>\
        <rect id='shape' x='0' y='0' width='30' height='30'/>\
        <clipPath id='self' clip-path='url(#self)'><rect width='50' height='50'/></clipPath>\
        <clipPath id='nothing'/>\
        </defs>";
    let c = |cp: &str, extra: &str| r100(&format!("{}<rect width='100' height='100' fill='red' clip-path='url(#{})' {}/>", defs, cp, extra));
    let sf = c("c", "");
    t.px(&sf, 50, 50, RED, 0, "circle: centre");
    t.px(&sf, 10, 10, NONE, 0, "circle: corner");
    let sf = r100(&format!("{}<rect x='20' width='60' height='100' fill='red' clip-path='url(#b)'/>", defs));
    t.px(&sf, 70, 50, RED, 0, "objectBoundingBox: right half");
    t.px(&sf, 30, 50, NONE, 0, "objectBoundingBox: left half cut");
    let sf = c("eo", "");
    t.px(&sf, 20, 20, RED, 0, "clip-rule evenodd: ring");
    t.px(&sf, 50, 50, NONE, 0, "clip-rule evenodd: hole");
    let sf = c("nest", "");
    t.px(&sf, 40, 50, RED, 0, "nested clip: both");
    t.px(&sf, 60, 50, NONE, 0, "nested clip: outside the inner");
    t.px(&sf, 10, 10, NONE, 0, "nested clip: outside the outer");
    let sf = c("t", "");
    t.px(&sf, 75, 75, RED, 0, "clip child transform");
    t.px(&sf, 25, 25, NONE, 0, "clip child transform: moved");
    let sf = c("u", "");
    t.px(&sf, 15, 15, RED, 0, "clip with use");
    let sf = c("missing", "");
    t.px(&sf, 10, 10, RED, 0, "missing clip path: unclipped");
    let sf = c("self", "");
    t.px(&sf, 10, 10, RED, 0, "self reference ends");
    let sf = c("nothing", "");
    t.px(&sf, 50, 50, NONE, 0, "empty clip path: nothing");
    let sf = r100(&format!("{}<g clip-path='url(#c)' opacity='0.5'><rect width='100' height='100' fill='red'/></g>", defs));
    t.alpha(&sf, 50, 50, 128, 1, "clip + group opacity");
    t.alpha(&sf, 10, 10, 0, 0, "clip + group opacity outside");
    let sf = r100(&format!("{}<g transform='translate(25 0)'><rect width='100' height='100' fill='red' clip-path='url(#c)'/></g>", defs));
    t.px(&sf, 75, 50, RED, 0, "clip follows the element's transform");
    t.px(&sf, 30, 50, NONE, 0, "clip moved with the element");
}

fn styles(t: &mut T) {
    t.group = "css";
    let sheet = "<style>\
        rect { fill: #0000ff }\
        .a { fill: #00ff00 }\
        #i { fill: #ff0000 }\
        g.box > rect { fill: #ffff00 }\
        g.deep rect { fill: #00ffff }\
        [data-k='v'] { fill: #ff00ff }\
        .x.y { fill: #800000 }\
        @media print { rect { fill: black } }\
        rect:hover { fill: black }\
        </style>";
    let one = |attrs: &str| r100(&format!("{}<rect width='100' height='100' {}/>", sheet, attrs));
    t.px(&one(""), 50, 50, BLUE, 0, "type selector");
    t.px(&one("class='a'"), 50, 50, 0xFF00_FF00, 0, "class");
    t.px(&one("class='a' id='i'"), 50, 50, RED, 0, "id beats class");
    t.px(&one("class='a' style='fill:#123456'"), 50, 50, 0xFF12_3456, 0, "style attribute beats the sheet");
    t.px(&one("fill='#123456' class='a'"), 50, 50, 0xFF00_FF00, 0, "the sheet beats presentation attributes");
    t.px(&one("data-k='v'"), 50, 50, 0xFFFF_00FF, 0, "attribute selector");
    t.px(&one("class='x y'"), 50, 50, 0xFF80_0000, 0, "two classes");
    t.px(&one("class='x'"), 50, 50, BLUE, 0, "two classes needed");
    let sf = r100(&format!("{}<g class='box'><rect width='100' height='100'/></g>", sheet));
    t.px(&sf, 50, 50, 0xFFFF_FF00, 0, "child combinator");
    let sf = r100(&format!("{}<g class='box'><g><rect width='100' height='100'/></g></g>", sheet));
    t.px(&sf, 50, 50, BLUE, 0, "child combinator: not a grandchild");
    let sf = r100(&format!("{}<g class='deep'><g><rect width='100' height='100'/></g></g>", sheet));
    t.px(&sf, 50, 50, 0xFF00_FFFF, 0, "descendant combinator");
    let sf = r100("<style>/* comment */ .q{fill:red;stroke:none!important} </style><rect class='q' width='100' height='100'/>");
    t.px(&sf, 50, 50, RED, 0, "comments, !important");
    let sf = r100("<rect width='100' height='100' style='fill: red; ; garbage; fill-opacity: 0.5'/>");
    t.px(&sf, 50, 50, 0x8080_0000, 1, "style attribute with junk");
    let sf = r100("<g fill='red'><rect width='100' height='100' fill='inherit'/></g>");
    t.px(&sf, 50, 50, RED, 0, "inherit");
    let sf = r100("<style type='text/foo'>rect{fill:red}</style><rect width='100' height='100' fill='blue'/>");
    t.px(&sf, 50, 50, BLUE, 0, "other style languages ignored");
    // display / visibility
    let sf = r100("<g display='none'><rect width='100' height='100' fill='red'/></g>");
    t.px(&sf, 50, 50, NONE, 0, "display none");
    let sf = r100("<g visibility='hidden'><rect width='50' height='100' fill='red'/><rect x='50' width='50' height='100' fill='red' visibility='visible'/></g>");
    t.px(&sf, 25, 50, NONE, 0, "visibility hidden");
    t.px(&sf, 75, 50, RED, 0, "visibility visible inside hidden");
    let sf = r100("<rect width='100' height='100' fill='red' style='display:none'/>");
    t.px(&sf, 50, 50, NONE, 0, "display none in style");
    let sf = r100("<defs><rect width='100' height='100' fill='red'/></defs><symbol><rect width='100' height='100' fill='red'/></symbol>");
    t.px(&sf, 50, 50, NONE, 0, "defs and symbol not drawn");
    let sf = r100("<switch><foo/><rect width='100' height='100' fill='red'/><rect width='100' height='100' fill='blue'/></switch>");
    t.px(&sf, 50, 50, RED, 0, "switch: the first drawable child");
    let sf = r100("<unknown><rect width='100' height='100' fill='red'/></unknown>");
    t.px(&sf, 50, 50, NONE, 0, "unknown element: not drawn");
}

fn uses(t: &mut T) {
    t.group = "use";
    let sf = r100("<defs><rect id='r' width='20' height='20'/></defs><use href='#r' x='50' y='50' fill='red'/>");
    t.px(&sf, 60, 60, RED, 0, "use x / y, fill inherited");
    t.px(&sf, 10, 10, NONE, 0, "use: original not drawn in defs");
    let sf = r100("<rect id='r' width='20' height='20' fill='blue'/><use xlink:href='#r' xmlns:xlink='http://www.w3.org/1999/xlink' x='50' fill='red'/>");
    t.px(&sf, 60, 10, BLUE, 0, "xlink:href; the element's own fill wins");
    let sf = r100("<defs><symbol id='s' viewBox='0 0 10 10'><rect width='5' height='5' fill='red'/></symbol></defs><use href='#s' width='100' height='100'/>");
    t.px(&sf, 40, 40, RED, 0, "symbol viewBox scales");
    t.px(&sf, 60, 60, NONE, 0, "symbol viewBox bound");
    let sf = r100("<defs><symbol id='s'><rect width='100' height='100' fill='red'/></symbol></defs><use href='#s' width='50' height='50'/>");
    t.px(&sf, 25, 25, RED, 0, "symbol clipped: inside");
    t.px(&sf, 75, 75, NONE, 0, "symbol clipped: outside");
    let sf = r100("<g id='a'><use href='#a'/><rect width='10' height='10' fill='red'/></g>");
    t.px(&sf, 5, 5, RED, 0, "self reference ends");
    let sf = r100("<g id='a'><use href='#b'/></g><g id='b'><use href='#a'/><rect width='10' height='10' fill='red'/></g>");
    t.px(&sf, 5, 5, RED, 0, "mutual references end");
    let sf = r100("<use href='http://example.com/x.svg#a'/><use href='#'/><use/>");
    t.px(&sf, 5, 5, NONE, 0, "external / empty references");
    let sf = r100("<defs><g id='g' transform='translate(10 10)'><rect width='10' height='10' fill='red'/></g></defs><use href='#g' transform='translate(30 30)'/>");
    t.px(&sf, 45, 45, RED, 0, "transforms of use and target");
}

fn images(t: &mut T) {
    t.group = "image";
    let svg = Svg::parse(FIXTURES.iter().find(|f| f.0 == "image").unwrap().1).unwrap();
    let sf = svg.rasterize(200, 200, &mut Opts::default()).unwrap();
    // the PNG: sky (120, 170, 230), ground (60, 140, 60), a transparent corner
    t.px(&sf, 50, 20, 0xFF78_AAE6, 3, "stretched: sky");
    t.px(&sf, 50, 85, 0xFF3C_8C3C, 3, "stretched: ground");
    t.px(&sf, 6, 6, NONE, 0, "transparent corner");
    t.px(&sf, 150, 10, NONE, 0, "meet: free band above");
    t.px(&sf, 150, 50, 0xFF78_AAE6, 3, "meet: the picture");
    let sf = r100("<image width='50' height='50' href='data:image/png;base64,!!!'/><image width='50' height='50' href='file.png'/>");
    t.px(&sf, 10, 10, NONE, 0, "broken / external images: nothing");
}

// a font engine of boxes: every character a box 0.5 em wide (0.1 em gaps), 0.7 em high above
// the baseline; bold boxes are a full em wide
struct Boxes;

impl Boxes {
    fn adv(s: &str, f: &img_svg::FontSpec, px: f32) -> f32 {
        let w = if f.weight >= 600 { 1.0 } else { 0.5 };
        s.chars().count() as f32 * px * w
    }
}

impl img_svg::TextHook for Boxes {
    fn outline(&mut self, path: &mut gfx::path::Path, s: &str, f: &img_svg::FontSpec, px: f32, x: f32, y: f32) -> f32 {
        let w = if f.weight >= 600 { 1.0 } else { 0.5 };
        for (i, ch) in s.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let x0 = x + i as f32 * px * w + px * 0.05;
            let x1 = x0 + px * w - px * 0.1;
            path.move_f(x0, y - px * 0.7);
            path.line_f(x1, y - px * 0.7);
            path.line_f(x1, y);
            path.line_f(x0, y);
            path.close();
        }
        Boxes::adv(s, f, px)
    }
    fn advance(&mut self, s: &str, f: &img_svg::FontSpec, px: f32) -> f32 {
        Boxes::adv(s, f, px)
    }
}

fn rt(body: &str) -> Surface {
    let svg = Svg::parse(doc(body).as_bytes()).unwrap();
    let mut b = Boxes;
    svg.rasterize(100, 100, &mut Opts { color: 0xFF00_0000, text: Some(&mut b) }).unwrap()
}

fn text(t: &mut T) {
    t.group = "text";
    let sf = r100("<text x='10' y='50' font-size='30' fill='red'>Hallo</text>");
    t.check(sf.px.iter().all(|&b| b == 0), "without a TextHook text is skipped");
    // "AB" at 20 px: two boxes of 10 px from x = 10
    let sf = rt("<text x='10' y='50' font-size='20' fill='red'>AB</text>");
    t.px(&sf, 15, 45, RED, 0, "start: first box");
    t.px(&sf, 25, 45, RED, 0, "start: second box");
    t.px(&sf, 35, 45, NONE, 0, "start: nothing after the text");
    t.px(&sf, 15, 52, NONE, 0, "start: nothing below the baseline");
    let sf = rt("<text x='50' y='50' font-size='20' fill='red' text-anchor='middle'>AB</text>");
    t.px(&sf, 45, 45, RED, 0, "middle: left of the anchor");
    t.px(&sf, 55, 45, RED, 0, "middle: right of the anchor");
    t.px(&sf, 35, 45, NONE, 0, "middle: before");
    let sf = rt("<text x='50' y='50' font-size='20' fill='red' text-anchor='end'>AB</text>");
    t.px(&sf, 35, 45, RED, 0, "end: before the anchor");
    t.px(&sf, 55, 45, NONE, 0, "end: nothing after the anchor");
    // blanks collapse; a tspan in bold (wider) and blue, dy moves the baseline
    let sf = rt("<text x='0' y='50' font-size='20' fill='red'>  A  <tspan font-weight='bold' fill='blue' dy='20'>B</tspan></text>");
    t.px(&sf, 5, 45, RED, 0, "leading blanks dropped");
    t.px(&sf, 15, 45, NONE, 0, "one blank between");
    t.px(&sf, 30, 65, BLUE, 0, "tspan: bold box, shifted by dy");
    t.px(&sf, 30, 45, NONE, 0, "tspan: not on the old baseline");
    // a tspan with x starts a new chunk; text-anchor per chunk
    let sf = rt("<text y='30' font-size='20' fill='red' text-anchor='middle'><tspan x='50'>AB</tspan><tspan x='50' y='70'>A</tspan></text>");
    t.px(&sf, 45, 25, RED, 0, "chunk 1 centred");
    t.px(&sf, 50, 65, RED, 0, "chunk 2 centred");
    t.px(&sf, 42, 65, NONE, 0, "chunk 2 one box");
    // transforms rotate the outlines; stroke and gradient paints
    let sf = rt("<text font-size='20' fill='red' transform='translate(50 10) rotate(90)'>AB</text>");
    t.px(&sf, 55, 15, RED, 0, "rotated: first box right of the origin");
    t.px(&sf, 55, 25, RED, 0, "rotated: second box further down");
    t.px(&sf, 45, 15, NONE, 0, "rotated: the ink side turned");
    let sf = rt("<text x='10' y='50' font-size='20' fill='none' stroke='red' stroke-width='2'>A</text>");
    t.px(&sf, 11, 45, RED, 0, "stroked text: the edge");
    t.px(&sf, 15, 42, NONE, 0, "stroked text: hollow");
    let sf = rt("<defs><clipPath id='c'><text x='0' y='60' font-size='40'>A</text></clipPath></defs><rect width='100' height='100' fill='red' clip-path='url(#c)'/>");
    t.px(&sf, 10, 45, RED, 0, "text as a clip path");
    t.px(&sf, 50, 45, NONE, 0, "text clip outside");
    let sf = rt("<text x='10' y='50' font-size='20' fill='red' display='none'>A</text><text x='30' y='50' font-size='20' fill='red' visibility='hidden'>A</text>");
    t.check(sf.px.iter().all(|&b| b == 0), "hidden text");
}

fn codec(t: &mut T) {
    t.group = "codec";
    use img_core::io;
    use img_core::*;
    t.eq(img_svg::register(), 0, "register");
    t.eq(io::format_from_path("a/b/Bild.SVG"), IMGF_SVG, "extension .svg");
    t.eq(io::format_from_path("x.svgz"), IMGF_SVG, "extension .svgz");
    let data = FIXTURES[0].1;
    t.eq(io::detect(data, 0), IMGF_SVG, "detect");
    let mut m = ImgMeta::default();
    t.eq(io::probe(data, Some(&mut m)), IMGF_SVG, "probe");
    t.eq((m.w, m.h), (200, 200), "probe: size");
    let mut m = ImgMeta::default();
    match io::decode_ex(data, None, Some(&mut m)) {
        Ok(sf) => {
            t.eq((sf.w, sf.h, sf.fmt), (200, 200, gfx::FMT_PARGB), "decode_ex: surface");
            t.eq(m.fmt, IMGF_SVG, "decode_ex: meta fmt");
            t.px(&sf, 50, 30, 0xFF3B_82F6, 0, "decode_ex: pixels");
        }
        Err(e) => t.check(false, &format!("decode_ex: {}", e)),
    }
    match io::decode(data) {
        Ok(sf) => t.eq((sf.w, sf.h), (200, 200), "classic decode"),
        Err(e) => t.check(false, &format!("classic decode: {}", e)),
    }
    let dec = ImgDec { flags: IMGD_STRAIGHT, ..ImgDec::default() };
    match io::decode_ex(b"<svg width='4' height='4'><rect width='4' height='4' fill='red' fill-opacity='0.5'/></svg>", Some(&dec), None) {
        Ok(sf) => t.px(&sf, 1, 1, 0x80FF_0000, 2, "straight output"),
        Err(e) => t.check(false, &format!("straight: {}", e)),
    }
    let dec = ImgDec { maxpx: 100, ..ImgDec::default() };
    t.check(io::decode_ex(data, Some(&dec), None).is_err(), "pixel budget");
    // svgz
    let gz = gzip::gzip_encode(data, 6);
    let dec = ImgDec { hint: IMGF_SVG, ..ImgDec::default() };
    match io::decode_ex(&gz, Some(&dec), None) {
        Ok(sf) => t.eq((sf.w, sf.h), (200, 200), "svgz with the hint"),
        Err(e) => t.check(false, &format!("svgz: {}", e)),
    }
    t.check(Svg::parse(&gz).is_ok(), "Svg::parse inflates gzip");
    t.check(Svg::parse(&gz[..gz.len() / 2]).is_err(), "truncated gzip");
    let list = io::filter_list(IFC_DECODE, 4096);
    t.check(list.windows(8).any(|w| w == b"svg;svgz"), "filter list");
    let fi = io::format_info(IMGF_SVG);
    t.check(fi.is_some_and(|f| f.name == "SVG" && f.mime == "image/svg+xml" && f.caps & IFC_DECODE != 0), "format_info");
    // rasterize at a size
    match img_svg::rasterize(data, 50, 0) {
        Ok(sf) => t.eq((sf.w, sf.h), (50, 50), "rasterize: height from the aspect ratio"),
        Err(e) => t.check(false, &format!("rasterize: {}", e)),
    }
    t.check(img_svg::rasterize(data, 70000, 10).is_err(), "rasterize: too wide");
    t.check(io::detect(b"<html></html>", 0) != IMGF_SVG, "html is no SVG");
}

fn malformed(t: &mut T) {
    t.group = "malformed";
    // every prefix of a fixture
    let data = FIXTURES.iter().find(|f| f.0 == "gradients").unwrap().1;
    let mut parsed = 0;
    for n in (0..data.len()).step_by(7) {
        if let Ok(s) = Svg::parse(&data[..n]) {
            parsed += 1;
            let _ = s.rasterize(48, 48, &mut Opts::default());
        }
    }
    t.check(parsed > 10, &format!("prefixes ({} parsed, no panic)", parsed));
    // mutations of every fixture: bytes replaced, removed, duplicated
    let mut g = Lcg(12345);
    let mut runs = 0;
    for (_, data) in FIXTURES {
        for _ in 0..60 {
            let mut d: Vec<u8> = data.to_vec();
            for _ in 0..1 + g.next() % 8 {
                if d.is_empty() {
                    break;
                }
                let at = g.next() as usize % d.len();
                match g.next() % 4 {
                    0 => d[at] = SYMS[g.next() as usize % SYMS.len()],
                    1 => {
                        d.remove(at);
                    }
                    2 => {
                        let b = d[at];
                        d.insert(at, b);
                    }
                    _ => d[at] = g.next() as u8,
                }
            }
            if let Ok(s) = Svg::parse(&d) {
                let _ = s.rasterize(40, 40, &mut Opts::default());
            }
            runs += 1;
        }
    }
    t.check(runs > 0, &format!("{} mutated files without a panic", runs));
    // deep nesting: deeper than the reader keeps
    let mut s = String::from("<svg>");
    for _ in 0..20000 {
        s.push_str("<g>");
    }
    s.push_str("<rect width='10' height='10' fill='red'/>");
    for _ in 0..20000 {
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    match Svg::parse(s.as_bytes()) {
        Ok(svg) => {
            t.check(svg.truncated(), "deep nesting: truncated");
            let _ = svg.rasterize(20, 20, &mut Opts::default());
        }
        Err(_) => t.check(false, "deep nesting parses"),
    }
    // many elements
    let mut s = String::from("<svg width='10' height='10'>");
    for _ in 0..260_000 {
        s.push_str("<g/>");
    }
    s.push_str("</svg>");
    let svg = Svg::parse(s.as_bytes()).unwrap();
    t.check(svg.truncated() && svg.element_count() <= 250_000, "element limit");
    // entity expansion (no recursion: each reference expands once)
    let mut s = String::from("<!DOCTYPE svg [<!ENTITY a 'aaaaaaaaaa'>");
    let names = ["b", "c", "d", "e", "f", "g", "h"];
    let mut prev = "a";
    for n in names {
        s.push_str(&format!("<!ENTITY {} '&{};&{};&{};&{};&{};&{};&{};&{};&{};&{};'>", n, prev, prev, prev, prev, prev, prev, prev, prev, prev, prev));
        prev = n;
    }
    s.push_str("]><svg><title>&h;&h;&h;</title></svg>");
    match Svg::parse(s.as_bytes()) {
        Ok(svg) => t.check(svg.title().len() < 10_000, "entities are not expanded recursively"),
        Err(_) => t.check(false, "entity file parses"),
    }
    // a use explosion: 10 levels of 10 references = 10^10 visits without a limit
    let mut s = String::from("<svg width='16' height='16'><defs><rect id='l0' width='1' height='1'/>");
    for k in 1..=10 {
        s.push_str(&format!("<g id='l{}'>", k));
        for _ in 0..10 {
            s.push_str(&format!("<use href='#l{}'/>", k - 1));
        }
        s.push_str("</g>");
    }
    s.push_str("</defs><use href='#l10'/></svg>");
    let t0 = plat::time_us();
    let svg = Svg::parse(s.as_bytes()).unwrap();
    let _ = svg.rasterize(16, 16, &mut Opts::default());
    let dt = (plat::time_us() - t0) / 1000;
    t.check(dt < 20_000, &format!("use explosion bounded ({} ms)", dt));
    // absurd numbers
    let cases = [
        "<rect width='1e308' height='1e308' fill='red'/>",
        "<rect width='nan' height='inf' fill='red'/>",
        "<rect width='100' height='100' transform='scale(1e300)' fill='red'/>",
        "<rect width='100' height='100' transform='scale(1e-300)' fill='red'/>",
        "<rect width='100' height='100' transform='matrix(1 2 2 4 0 0)' fill='red'/>",
        "<circle r='1e9' stroke='red' stroke-width='1e9'/>",
        "<path d='M0 0 L1e9 1e9 L-1e9 1e9 Z' fill='red'/>",
        "<path d='M0 0 A1e9 1e-9 45 1 1 100 100' stroke='red'/>",
        "<line x2='100' stroke='red' stroke-width='5' stroke-dasharray='0.000001 0.000001'/>",
        "<line x2='100' stroke='red' stroke-width='5' stroke-dasharray='1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33'/>",
        "<text x='10' y='10' font-size='1e9'>x</text>",
        "<rect width='100' height='100' rx='-5' ry='1e300' fill='red'/>",
        "<polygon points='1e400 5 nan 2' fill='red'/>",
        "<rect width='100' height='100' stroke-width='-5' stroke='red' stroke-miterlimit='-1'/>",
        "<svg width='0' height='0'><rect width='5' height='5'/></svg>",
        "<svg viewBox='0 0 1e-300 1e-300'><rect width='5' height='5'/></svg>",
    ];
    for c in cases {
        let _ = r100(c);
    }
    t.check(true, "absurd numbers");
    // many gradient stops, a huge sheet
    let mut s = String::from("<linearGradient id='g'>");
    for k in 0..5000 {
        s.push_str(&format!("<stop offset='{}' stop-color='#{:06x}'/>", k as f64 / 5000.0, k * 3301 & 0xFFFFFF));
    }
    s.push_str("</linearGradient><rect width='100' height='100' fill='url(#g)'/>");
    let _ = r100(&s);
    let mut s = String::from("<style>");
    for k in 0..6000 {
        s.push_str(&format!("g g g g g g g g .c{} {{ fill: red }} ", k));
    }
    s.push_str("</style>");
    for _ in 0..200 {
        s.push_str("<g>");
    }
    s.push_str("<rect width='10' height='10'/>");
    for _ in 0..200 {
        s.push_str("</g>");
    }
    let t0 = plat::time_us();
    let _ = r100(&s);
    let dt = (plat::time_us() - t0) / 1000;
    t.check(dt < 20_000, &format!("huge style sheet bounded ({} ms)", dt));
    // broken markup
    let broken: [&[u8]; 10] = [
        b"<svg><rect width='10' height='10' fill='red'",
        b"<svg><rect width=10 height=10 fill=red /></svg>",
        b"<svg><<<>>></svg>",
        b"<svg><g></rect></g></svg>",
        b"<svg>&unknown; &#xFFFFFFFF; &#0; &#;</svg>",
        b"<svg><![CDATA[ unterminated",
        b"<svg><!-- unterminated",
        b"<svg width='\xFF\xFE'/>",
        b"<svg><style>rect{fill:red</style><rect width='5' height='5'/></svg>",
        b"<svg><rect width='5' height='5' style='fill:url(#'/></svg>",
    ];
    for b in broken {
        if let Ok(s) = Svg::parse(b) {
            let _ = s.rasterize(10, 10, &mut Opts::default());
        }
    }
    let sf = r(&String::from_utf8_lossy(broken[1]), 10, 10);
    t.px(&sf, 5, 5, RED, 0, "unquoted attributes");
    let sf = r(&String::from_utf8_lossy(broken[0]), 10, 10);
    t.px(&sf, 5, 5, RED, 0, "unterminated tag");
}

/// run every group -> the number of failures
pub fn run() -> i32 {
    let mut t = T { group: "", ok: 0, failed: 0 };
    let t0 = plat::time_us();
    img_all::use_core_codecs(); // the PNG of the embedded image
    parse_and_sizes(&mut t);
    colors(&mut t);
    paths(&mut t);
    transforms(&mut t);
    viewbox(&mut t);
    strokes(&mut t);
    gradients(&mut t);
    opacity(&mut t);
    clipping(&mut t);
    styles(&mut t);
    uses(&mut t);
    images(&mut t);
    text(&mut t);
    codec(&mut t);
    malformed(&mut t);
    let dt = (plat::time_us() - t0) / 1000;
    out(&format!("svgtest --test unit: {} passed, {} failed ({} ms)\n", t.ok, t.failed, dt));
    t.failed as i32
}
