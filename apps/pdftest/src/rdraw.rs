// rdraw -- the check groups of the renderer (pdf::read, Document::render): pages written here
// byte by byte, drawn at 72 dpi (a point is a pixel) and checked pixel by pixel.  No check
// depends on the fonts of the system: text is drawn in Type 3 fonts, in a Type 1 font built
// here and in the fonts the writer's sample embeds.  tools/pdf_render_check.py compares whole
// pages with Poppler.
//
//   render_basic     page sizes (boxes, rotation, dpi), the paper, regions, fills and fill
//                    rules, strokes (pixel aligned, dashes, hairlines), clipping, cm, q / Q
//   render_color     every colour space, CMYK as inks, functions of type 0, 2, 3 and 4
//   render_image     bit depths, Decode, Indexed, stencil and colour-key masks, soft masks,
//                    inline images, JPEG, CCITT Group 3 and 4, enlarged and shrunk images
//   render_shade     axial, radial, function-based and mesh shadings, shading patterns, tiling
//                    patterns coloured and uncoloured
//   render_transp    alpha, blend modes against the page and other paint, soft masks, groups
//   render_text      Type 3 glyphs and render modes, a Type 1 font (subroutines, seac), the
//                    sample's embedded fonts
//   render_annot     annotation appearances, hidden annotations, states, optional content
//   render_hostile   recursion in forms, patterns, glyphs and soft masks, huge pages, images
//                    and meshes, broken font programs and image data, mutated files

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use pdf::read::{Document, Error, RenderOptions};

use crate::rcheck::{cat, flate, stream, Mk};
use crate::T;

pub const GROUPS: &[(&str, fn(&mut T))] = &[
    ("render_basic", render_basic),
    ("render_color", render_color),
    ("render_image", render_image),
    ("render_shade", render_shade),
    ("render_transp", render_transp),
    ("render_text", render_text),
    ("render_annot", render_annot),
    ("render_hostile", render_hostile),
];

// ------------------------------------------------------------------ pages and pixels

/// one page of 100 x 100 points: content, resources (inside << >>), objects from 10 on, more
/// page entries
fn page(content: &[u8], res: &str, extra: &[(u32, Vec<u8>)], page_extra: &str) -> Vec<u8> {
    let mut m = Mk::new();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R >>")
        .add(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .add(3, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << {res} >> {page_extra} >>"))
        .add(4, flate("", content));
    for (n, b) in extra {
        m.add(*n, b);
    }
    m.table("/Root 1 0 R")
}

/// a page drawn: RGB over the paper, and the alpha before it
struct Px {
    w: usize,
    h: usize,
    rgb: Vec<[u8; 3]>,
    alpha: Vec<u8>,
}

impl Px {
    fn of(sf: &gfx::Surface) -> Px {
        let (w, h) = (sf.w as usize, sf.h as usize);
        let mut rgb = Vec::with_capacity(w * h);
        let mut alpha = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let p = &sf.px[y * sf.stride as usize + x * 4..][..4];
                let ia = 255 - p[3] as u32;
                rgb.push([(p[2] as u32 + ia).min(255) as u8, (p[1] as u32 + ia).min(255) as u8, (p[0] as u32 + ia).min(255) as u8]);
                alpha.push(p[3]);
            }
        }
        Px { w, h, rgb, alpha }
    }

    /// the pixel under page point (x, y) of a 100-point page at 72 dpi
    fn at(&self, x: f64, y: f64) -> [u8; 3] {
        let (dx, dy) = (x as usize, (self.h as f64 - y) as usize);
        self.rgb[dy.min(self.h - 1) * self.w + dx.min(self.w - 1)]
    }

    fn dev(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb[y.min(self.h - 1) * self.w + x.min(self.w - 1)]
    }
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= tol)
}

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn draw_opt(t: &mut T, name: &str, data: Vec<u8>, opt: &RenderOptions) -> Option<Px> {
    let doc = t.ok(&format!("{name}: opens"), Document::open(data).map_err(|e| format!("{e}")))?;
    let sf = t.ok(&format!("{name}: draws"), doc.render(0, opt).map_err(|e| format!("{e}")))?;
    Some(Px::of(&sf))
}

fn draw(t: &mut T, name: &str, data: Vec<u8>) -> Option<Px> {
    draw_opt(t, name, data, &RenderOptions::default())
}

/// draw a page and check pixels at page points
fn expect(t: &mut T, name: &str, data: Vec<u8>, points: &[(f64, f64, [u8; 3])], tol: i32) {
    let Some(px) = draw(t, name, data) else { return };
    for &(x, y, c) in points {
        let got = px.at(x, y);
        if near(got, c, tol) {
            t.check(name, true);
        } else {
            t.eq(&format!("{name}: the pixel at {x}, {y}"), got, c);
        }
    }
}

// ------------------------------------------------------------------ render_basic

fn render_basic(t: &mut T) {
    // sizes
    let sized = |mb: &str, more: &str| -> Vec<u8> {
        let mut m = Mk::new();
        m.add(1, "<< /Type /Catalog /Pages 2 0 R >>")
            .add(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
            .add(3, format!("<< /Type /Page /Parent 2 0 R /MediaBox {mb} {more} /Contents 4 0 R >>"))
            .add(4, flate("", b"1 0 0 rg 0 0 20 20 re f"));
        m.table("/Root 1 0 R")
    };
    for (mb, more, dpi, want) in [
        ("[0 0 100 100]", "", 72.0, (100, 100)),
        ("[0 0 100 100]", "", 144.0, (200, 200)),
        ("[0 0 100.5 50]", "", 72.0, (101, 50)),
        ("[0 0 100 50]", "/Rotate 90", 72.0, (50, 100)),
        ("[0 0 100 100]", "/CropBox [10 10 60 40]", 72.0, (50, 30)),
        ("[0 0 100 100]", "/UserUnit 2", 72.0, (200, 200)),
    ] {
        if let Some(doc) = t.ok("sized page opens", Document::open(sized(mb, more)).map_err(|e| format!("{e}"))) {
            t.eq(&format!("page_pixels {mb} {more} at {dpi}"), doc.page_pixels(0, dpi).ok(), Some(want));
        }
    }
    // /Rotate 90: the corner at the origin goes to the top left
    if let Some(px) = draw(t, "rotated", sized("[0 0 100 100]", "/Rotate 90")) {
        t.eq("rotate 90: the square at the top left", px.dev(10, 10), RED);
        t.eq("rotate 90: the bottom left is empty", px.dev(10, 90), WHITE);
    }
    if let Some(px) = draw(t, "rotated 180", sized("[0 0 100 100]", "/Rotate 180")) {
        t.eq("rotate 180: the square at the top right", px.dev(90, 10), RED);
    }
    if let Some(px) = draw(t, "media box off the origin", sized("[-50 -50 50 50]", "")) {
        t.eq("media box off the origin: the square in the middle", px.dev(55, 45), RED);
        t.eq("media box off the origin: the corner is empty", px.dev(45, 55), WHITE);
    }
    if let Some(px) = draw(t, "crop box", sized("[0 0 100 100]", "/CropBox [10 10 60 40]")) {
        t.eq("crop box: the square from its corner", px.dev(5, 25), RED);
        t.eq("crop box: beyond the square", px.dev(15, 15), WHITE);
    }
    // the paper, regions
    let one = page(b"0 0 1 rg 10 10 30 20 re f", "", &[], "");
    if let Some(px) = draw_opt(t, "no paper", one.clone(), &RenderOptions { background: None, ..Default::default() }) {
        t.eq("no paper: an empty pixel is transparent", px.alpha[0], 0);
        t.eq("no paper: a painted one is opaque", px.alpha[85 * 100 + 20], 255);
    }
    if let Some(px) = draw_opt(t, "a region", one.clone(), &RenderOptions { region: Some([10, 70, 30, 20]), ..Default::default() }) {
        t.eq("a region: its size", (px.w, px.h), (30, 20));
        t.check("a region: all of it is the rectangle", px.rgb.iter().all(|&c| c == BLUE));
    }
    if let Some(doc) = t.ok("region doc", Document::open(one.clone()).map_err(|e| format!("{e}"))) {
        t.check("a region outside the page is an error", matches!(doc.render(0, &RenderOptions { region: Some([200, 0, 10, 10]), ..Default::default() }), Err(Error::Limit(_))));
        t.check("a page that does not exist is an error", matches!(doc.render(3, &RenderOptions::default()), Err(Error::NoPage)));
    }
    // a rectangle fills exactly its pixels
    if let Some(px) = draw(t, "rectangle", one) {
        let inside = (70..90).all(|y| (10..40).all(|x| px.dev(x, y) == BLUE));
        let around = (69..91).all(|y| px.dev(9, y) == WHITE && px.dev(40, y) == WHITE) && (9..41).all(|x| px.dev(x, 69) == WHITE && px.dev(x, 90) == WHITE);
        t.check("rectangle: every pixel inside", inside);
        t.check("rectangle: none around it", around);
    }
    // fill rules: two squares the same way round, then a ring
    let squares = b"0 0 m 80 0 l 80 80 l 0 80 l h 20 20 m 60 20 l 60 60 l 20 60 l h";
    expect(t, "non-zero", page(&cat(&[b"1 0 0 rg ", squares, b" f"]), "", &[], ""), &[(40.0, 40.0, RED), (10.0, 10.0, RED)], 0);
    expect(t, "even-odd", page(&cat(&[b"1 0 0 rg ", squares, b" f*"]), "", &[], ""), &[(40.0, 40.0, WHITE), (10.0, 10.0, RED)], 0);
    // curves: a circle covers about pi r^2
    let circle = b"0 g 50 10 m 72.1 10 90 27.9 90 50 c 90 72.1 72.1 90 50 90 c 27.9 90 10 72.1 10 50 c 10 27.9 27.9 10 50 10 c f";
    if let Some(px) = draw(t, "circle", page(circle, "", &[], "")) {
        let ink: f64 = px.rgb.iter().map(|c| (255 - c[0]) as f64 / 255.0).sum();
        let area = core::f64::consts::PI * 1600.0;
        t.check(&format!("circle: covers {ink:.0} of {area:.0} pixels"), (ink - area).abs() < area * 0.01);
    }
    // strokes: thin rules on whole pixels, dashes, hairlines, caps
    if let Some(px) = draw(t, "thin rule", page(b"0 g 0.4 w 10 50 m 90 50 l S", "", &[], "")) {
        t.eq("thin rule: one row of pixels", (px.dev(50, 49), px.dev(50, 50), px.dev(50, 51)), (WHITE, BLACK, WHITE));
        t.eq("thin rule: its ends", (px.dev(9, 50), px.dev(10, 50), px.dev(89, 50), px.dev(90, 50)), (WHITE, BLACK, BLACK, WHITE));
    }
    if let Some(px) = draw(t, "hairline", page(b"0 g 0 w 50 10 m 50 90 l S", "", &[], "")) {
        t.eq("hairline: one column of pixels", (px.dev(49, 50), px.dev(50, 50), px.dev(51, 50)), (WHITE, BLACK, WHITE));
    }
    expect(t, "dashes", page(b"0 g 4 w [10 10] 0 d 0 50 m 100 50 l S", "", &[], ""), &[(5.0, 50.0, BLACK), (15.0, 50.0, WHITE), (25.0, 50.0, BLACK), (95.0, 50.0, WHITE)], 0);
    expect(t, "dash phase", page(b"0 g 4 w [10 10] 10 d 0 50 m 100 50 l S", "", &[], ""), &[(5.0, 50.0, WHITE), (15.0, 50.0, BLACK)], 0);
    expect(t, "square caps", page(b"0 g 10 w 2 J 20 50 m 80 50 l S", "", &[], ""), &[(17.0, 50.0, BLACK), (83.0, 50.0, BLACK), (88.0, 50.0, WHITE)], 0);
    expect(t, "butt caps", page(b"0 g 10 w 0 J 20 50 m 80 50 l S", "", &[], ""), &[(17.0, 50.0, WHITE), (50.0, 54.0, BLACK)], 0);
    // a turned stroke: 45 degrees, 10 wide
    expect(t, "turned stroke", page(b"0 g 10 w 10 10 m 90 90 l S", "", &[], ""), &[(50.0, 50.0, BLACK), (52.0, 48.0, BLACK), (60.0, 40.0, WHITE)], 0);
    // a stroke under a non-uniform matrix is wide in user space
    expect(t, "scaled stroke", page(b"0 g q 4 0 0 1 0 0 cm 2 w 5 20 m 5 80 l S Q", "", &[], ""), &[(16.0, 50.0, BLACK), (23.0, 50.0, BLACK), (27.0, 50.0, WHITE)], 0);
    // clipping
    expect(t, "clip rectangle", page(b"10 10 50 50 re W n 0 0 1 rg 0 0 100 100 re f", "", &[], ""), &[(20.0, 20.0, BLUE), (70.0, 70.0, WHITE)], 0);
    expect(t, "clip circle", page(&cat(&[&circle[..circle.len() - 1], b"W n 1 0 0 rg 0 0 100 100 re f"]), "", &[], ""), &[(50.0, 50.0, RED), (13.0, 13.0, WHITE)], 0);
    expect(t, "clip even-odd", page(&cat(&[squares, b" W* n 1 0 0 rg 0 0 100 100 re f"]), "", &[], ""), &[(40.0, 40.0, WHITE), (10.0, 10.0, RED), (90.0, 90.0, WHITE)], 0);
    expect(t, "clips inside each other", page(b"0 0 50 100 re W n 0 50 100 50 re W n 0 1 0 rg 0 0 100 100 re f", "", &[], ""), &[(25.0, 75.0, GREEN), (75.0, 75.0, WHITE), (25.0, 25.0, WHITE)], 0);
    expect(t, "Q ends a clip", page(b"q 10 10 20 20 re W n Q 1 0 0 rg 0 0 100 100 re f", "", &[], ""), &[(80.0, 80.0, RED)], 0);
    expect(t, "cm", page(b"q 1 0 0 1 50 50 cm 0 g 0 0 10 10 re f Q 1 0 0 rg 0 0 10 10 re f", "", &[], ""), &[(55.0, 55.0, BLACK), (5.0, 5.0, RED), (45.0, 45.0, WHITE)], 0);
    expect(t, "the colour comes back with Q", page(b"0 0 1 rg q 1 0 0 rg Q 0 0 100 100 re f", "", &[], ""), &[(50.0, 50.0, BLUE)], 0);
    expect(t, "closing paths s and b", page(b"0 g 4 w 1 0 0 rg 20 20 m 80 20 l 80 80 l b", "", &[], ""), &[(60.0, 40.0, RED), (50.0, 50.0, BLACK)], 0);
}

// ------------------------------------------------------------------ render_color

fn render_color(t: &mut T) {
    let patch = |op: &str| -> Vec<u8> { format!("{op} 0 0 100 100 re f").into_bytes() };
    for (op, want) in [
        ("0.5 g", [128, 128, 128]),
        ("0 1 0 rg", GREEN),
        ("0 0 0 0 k", WHITE),
        ("1 0 0 0 k", [0, 173, 239]),
        ("0 1 0 0 k", [236, 0, 140]),
        ("0 0 1 0 k", [255, 242, 0]),
        ("0 0 0 1 k", [35, 31, 32]),
        ("1 1 1 1 k", BLACK),
    ] {
        expect(t, &format!("device colour {op}"), page(patch(op).as_slice(), "", &[], ""), &[(50.0, 50.0, want)], 1);
    }
    let cs = "/ColorSpace << /I [/Indexed /DeviceRGB 1 <ff0000 0000ff>] /S [/Separation /Spot /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] \
              /P [/Separation /Gray /DeviceRGB 10 0 R] /Q [/Separation /Choice /DeviceRGB 11 0 R] /R [/Separation /Bits /DeviceGray 12 0 R] \
              /N [/DeviceN [/A /B] /DeviceRGB 13 0 R] /T [/Separation /Arr /DeviceRGB [<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [0] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>]] \
              /U [/Separation /Stitch /DeviceGray << /FunctionType 3 /Domain [0 1] /Functions [<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] /Bounds [0.5] /Encode [0 1 0 1] >>] \
              /V [/Separation /Sampled /DeviceRGB 14 0 R] \
              /L [/Lab << /WhitePoint [0.9505 1 1.089] /Range [-100 100 -100 100] >>] /C [/CalRGB << /WhitePoint [0.9505 1 1.089] /Matrix [0.4124 0.2126 0.0193 0.3576 0.7152 0.1192 0.1805 0.0722 0.9505] >>] \
              /G [/CalGray << /WhitePoint [0.9505 1 1.089] /Gamma 2.2 >>] /None [/Separation /None /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] \
              /All [/Separation /All /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] >>";
    let extra = vec![
        (10, stream("/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]", b"{ dup dup }")),
        (11, stream("/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]", b"{ 0.5 gt { 1 0 0 } { 0 0 1 } ifelse }")),
        (12, stream("/FunctionType 4 /Domain [0 1] /Range [0 1]", b"{ 4 mul cvi 2 mod dup 0 eq { pop 1 } { pop 0 } ifelse }")),
        (13, stream("/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1]", b"{ exch 2 copy add 2 div 3 1 roll exch 3 -1 roll }")),
        (14, stream("/FunctionType 0 /Domain [0 1] /Range [0 1 0 1 0 1] /Size [3] /BitsPerSample 8", &[255, 0, 0, 0, 255, 0, 0, 0, 255])),
    ];
    for (op, want, tol) in [
        ("/I cs 1 sc", BLUE, 0),
        ("/I cs 0 sc", RED, 0),
        ("/S cs 1 scn", RED, 1),
        ("/S cs 0.5 scn", [255, 128, 128], 1),
        ("/P cs 0.25 scn", [64, 64, 64], 1),
        ("/Q cs 0.7 scn", RED, 0),
        ("/Q cs 0.3 scn", BLUE, 0),
        ("/R cs 0.3 scn", BLACK, 0),
        ("/R cs 0.6 scn", WHITE, 0),
        ("/N cs 0.2 0.8 scn", [51, 204, 128], 1),
        ("/T cs 0.5 scn", [128, 0, 128], 1),
        ("/U cs 0.25 scn", [128, 128, 128], 1),
        ("/U cs 0.75 scn", [128, 128, 128], 1),
        ("/V cs 0.5 scn", GREEN, 0),
        ("/V cs 0.25 scn", [128, 128, 0], 1),
        ("/L cs 100 0 0 scn", WHITE, 2),
        ("/L cs 0 0 0 scn", BLACK, 2),
        ("/C cs 1 0 0 scn", RED, 3),
        ("/C cs 1 1 1 scn", WHITE, 3),
        ("/C cs 0.5 0.5 0.5 scn", [188, 188, 188], 3),
        ("/G cs 1 sc", WHITE, 1),
        ("/None cs 1 sc", WHITE, 0),
        ("/All cs 1 sc", BLACK, 0),
        ("0 1 0 RG /S CS 1 SCN 0 0 1 rg", BLUE, 0),
    ] {
        expect(t, &format!("colour space: {op}"), page(patch(op).as_slice(), cs, &extra, ""), &[(50.0, 50.0, want)], tol);
    }
    // a stroke in a Separation colour
    expect(t, "stroke colour space", page(b"/S CS 1 SCN 10 w 0 50 m 100 50 l S", cs, &extra, ""), &[(50.0, 50.0, RED), (50.0, 20.0, WHITE)], 1);
    // an unknown colour space keeps the colour before it
    expect(t, "unknown colour space", page(b"0 0 1 rg /Nope cs 0 0 100 100 re f", cs, &extra, ""), &[(50.0, 50.0, BLUE)], 0);
}

// ------------------------------------------------------------------ render_image

fn img(dict: &str, data: &[u8]) -> Vec<u8> {
    stream(&format!("/Type /XObject /Subtype /Image {dict}"), data)
}

fn render_image(t: &mut T) {
    // 2 x 2: red green / blue white, drawn 80 x 80 (20 times: the nearest sample)
    let quad = [255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let xo = "/XObject << /I 10 0 R >>";
    let corners = |tol| -> Vec<(f64, f64, [u8; 3])> {
        let _ = tol;
        vec![(20.0, 80.0, RED), (80.0, 80.0, GREEN), (20.0, 20.0, BLUE), (80.0, 20.0, WHITE)]
    };
    expect(t, "image enlarged", page(b"q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", &quad))], ""), &corners(0), 0);
    expect(t, "image enlarged smoothly", page(b"q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Interpolate true", &quad))], ""), &[(14.0, 86.0, RED), (86.0, 14.0, WHITE)], 2);
    // shrunk: 80 x 80 pixels of 1-pixel stripes into 40 x 40: the average
    let stripes: Vec<u8> = (0..80 * 80).map(|i| if i % 2 == 0 { 0 } else { 255 }).collect();
    expect(t, "image shrunk", page(b"q 40 0 0 40 30 30 cm /I Do Q", xo, &[(10, img("/Width 80 /Height 80 /ColorSpace /DeviceGray /BitsPerComponent 8", &stripes))], ""), &[(50.0, 50.0, [128, 128, 128])], 2);
    // bit depths and Decode
    expect(t, "1-bit image", page(b"q 80 0 0 10 10 50 cm /I Do Q", xo, &[(10, img("/Width 8 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 1", &[0b1010_1010]))], ""), &[(15.0, 55.0, WHITE), (25.0, 55.0, BLACK), (85.0, 55.0, BLACK)], 0);
    expect(t, "Decode [1 0]", page(b"q 80 0 0 10 10 50 cm /I Do Q", xo, &[(10, img("/Width 8 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 1 /Decode [1 0]", &[0b1010_1010]))], ""), &[(15.0, 55.0, BLACK), (25.0, 55.0, WHITE)], 0);
    expect(t, "2-bit image", page(b"q 80 0 0 10 10 50 cm /I Do Q", xo, &[(10, img("/Width 4 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 2", &[0b00_01_10_11]))], ""), &[(15.0, 55.0, BLACK), (35.0, 55.0, [85, 85, 85]), (55.0, 55.0, [170, 170, 170]), (75.0, 55.0, WHITE)], 1);
    expect(t, "4-bit Indexed", page(b"q 80 0 0 10 10 50 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 1 /ColorSpace [/Indexed /DeviceRGB 2 <ff0000 00ff00 0000ff>] /BitsPerComponent 4", &[0x21]))], ""), &[(30.0, 55.0, BLUE), (70.0, 55.0, GREEN)], 0);
    expect(t, "16-bit image", page(b"q 80 0 0 10 10 50 cm /I Do Q", xo, &[(10, img("/Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 16", &[0xFF, 0xFF, 0x80, 0x00, 0, 0]))], ""), &[(50.0, 55.0, [255, 128, 0])], 1);
    expect(t, "CMYK image", page(b"q 80 0 0 10 10 50 cm /I Do Q", xo, &[(10, img("/Width 1 /Height 1 /ColorSpace /DeviceCMYK /BitsPerComponent 8", &[255, 0, 0, 0]))], ""), &[(50.0, 55.0, [0, 173, 239])], 1);
    // masks: a stencil painted in the fill colour, a colour key, an explicit mask, a soft mask
    expect(t, "stencil mask", page(b"0 1 0 rg q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 2 /ImageMask true /BitsPerComponent 1", &[0b0100_0000, 0b1000_0000]))], ""), &[(20.0, 80.0, GREEN), (80.0, 80.0, WHITE), (20.0, 20.0, WHITE), (80.0, 20.0, GREEN)], 0);
    expect(t, "stencil mask Decode [1 0]", page(b"0 1 0 rg q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 2 /ImageMask true /BitsPerComponent 1 /Decode [1 0]", &[0b0100_0000, 0b1000_0000]))], ""), &[(20.0, 80.0, WHITE), (80.0, 80.0, GREEN)], 0);
    expect(t, "colour key", page(b"0 0 0 rg 0 0 100 100 re f q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Mask [250 255 0 5 0 5]", &quad))], ""), &[(20.0, 80.0, BLACK), (80.0, 80.0, GREEN), (80.0, 20.0, WHITE)], 0);
    let mask = img("/Width 2 /Height 2 /ImageMask true /BitsPerComponent 1", &[0b1000_0000, 0b0000_0000]);
    expect(t, "explicit mask", page(b"0 0 0 rg 0 0 100 100 re f q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Mask 11 0 R", &quad)), (11, mask)], ""), &[(20.0, 80.0, BLACK), (80.0, 80.0, GREEN), (20.0, 20.0, BLUE)], 0);
    let soft = img("/Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8", &[0, 128]);
    expect(t, "soft mask", page(b"0 0 0 rg 0 0 100 100 re f q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 11 0 R", &[255, 255])), (11, soft)], ""), &[(20.0, 50.0, BLACK), (80.0, 50.0, [128, 128, 128])], 1);
    // an inline image with abbreviations and a filter
    expect(t, "inline image", page(b"q 80 0 0 80 10 10 cm BI /W 2 /H 2 /CS /RGB /BPC 8 /F /AHx ID ff000000ff000000ffffffff> EI Q", "", &[], ""), &corners(0), 0);
    // turned: the middle of the image is where the matrix puts it
    expect(t, "turned image", page(b"q 0 80 -80 0 90 10 cm /I Do Q", xo, &[(10, img("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", &quad))], ""), &[(30.0, 30.0, RED), (30.0, 70.0, GREEN), (70.0, 30.0, BLUE), (70.0, 70.0, WHITE)], 0);
    // JPEG through img_jpeg
    let mut sf = gfx::Surface::new(16, 16, gfx::FMT_PARGB);
    for p in sf.px.chunks_exact_mut(4) {
        p.copy_from_slice(&[40, 160, 200, 255]);
    }
    if let Some(jpeg) = img_jpeg::encode(&sf, 95) {
        expect(t, "JPEG image", page(b"q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 16 /Height 16 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", &jpeg))], ""), &[(50.0, 50.0, [200, 160, 40])], 6);
    }
    // CCITT: two rows of 8, four white then four black, as Group 3 (1-D) and Group 4
    for (name, k, data) in [("CCITT Group 3", 0, [0xB7u8, 0x6C]), ("CCITT Group 4", -1, [0x36, 0xF0])] {
        expect(
            t,
            name,
            page(b"q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img(&format!("/Width 8 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K {k} /Columns 8 /Rows 2 >>"), &data))], ""),
            &[(20.0, 70.0, WHITE), (80.0, 70.0, BLACK), (20.0, 30.0, WHITE), (80.0, 30.0, BLACK)],
            0,
        );
    }
    expect(
        t,
        "CCITT BlackIs1",
        page(b"q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img("/Width 8 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 /BlackIs1 true >>", &[0x36, 0xF0]))], ""),
        &[(20.0, 70.0, BLACK), (80.0, 70.0, WHITE)],
        0,
    );
    // JPEG 2000 (the fixtures below): the top half (40, 160, 200), the bottom half (200, 40, 90)
    let jpx = |dict: &str, data: &[u8]| page(b"q 80 0 0 80 10 10 cm /I Do Q", xo, &[(10, img(dict, data))], "");
    let (top, bottom) = ([40, 160, 200], [200, 40, 90]);
    expect(t, "JPX codestream as DeviceRGB", jpx("/Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /JPXDecode", J2K_RGB), &[(50.0, 70.0, top), (50.0, 30.0, bottom)], 1);
    expect(t, "JPX colour space from the JP2 header", jpx("/Width 8 /Height 8 /Filter /JPXDecode", JP2_RGB), &[(50.0, 70.0, top), (50.0, 30.0, bottom)], 1);
    expect(t, "JPX grey from the JP2 header", jpx("/Width 8 /Height 8 /Filter /JPXDecode", JP2_GRAY), &[(50.0, 70.0, [60, 60, 60]), (50.0, 30.0, [220, 220, 220])], 1);
    expect(t, "JPX with SMaskInData", jpx("/Width 8 /Height 8 /Filter /JPXDecode /SMaskInData 1", JP2_RGBA), &[(50.0, 70.0, top), (50.0, 30.0, WHITE)], 1);
    expect(t, "JPX opacity ignored without SMaskInData", jpx("/Width 8 /Height 8 /Filter /JPXDecode", JP2_RGBA), &[(50.0, 70.0, top), (50.0, 30.0, bottom)], 1);
    // (heavily compressed: the colours OpenJPEG decodes at those pixels)
    expect(t, "JPX lossy (9/7)", jpx("/Width 16 /Height 16 /ColorSpace /DeviceRGB /Filter /JPXDecode", JP2_LOSSY), &[(50.0, 75.0, [25, 181, 214]), (50.0, 25.0, [235, 47, 75])], 2);
    expect(t, "JPX size from the codestream", jpx("/Width 3 /Height 5 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /JPXDecode", J2K_RGB), &[(50.0, 70.0, top), (50.0, 30.0, bottom)], 1);
    // 4-bit samples (1 above, 5 below): indices of Indexed and the values a colour-key mask names,
    // as they are coded (entry 15, green, would be the samples scaled to 8 bits)
    let indexed = "/Width 8 /Height 8 /ColorSpace [/Indexed /DeviceRGB 15 <000000FF00000000000000000000000000FF00000000000000000000000000000000000000000000000000000000FF00>] /Filter /JPXDecode";
    expect(t, "JPX 4-bit indices", jpx(indexed, J2K_GRAY4), &[(50.0, 70.0, [255, 0, 0]), (50.0, 30.0, [0, 0, 255])], 0);
    expect(t, "JPX colour-key mask on 4-bit samples", jpx("/Width 8 /Height 8 /ColorSpace /DeviceGray /Mask [5 5] /Filter /JPXDecode", J2K_GRAY4), &[(50.0, 70.0, [17, 17, 17]), (50.0, 30.0, WHITE)], 0);
    // a damaged one leaves the page alone
    expect(t, "JPX damaged", jpx("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /JPXDecode", b"junk"), &[(50.0, 50.0, WHITE)], 0);
    expect(t, "JPX cut short", jpx("/Width 8 /Height 8 /ColorSpace /DeviceRGB /Filter /JPXDecode", &J2K_RGB[..40]), &[(50.0, 50.0, WHITE)], 0);
    // a header that claims 2^20 x 2^20 pixels (one tile) is refused before anything is allocated
    let mut huge = J2K_RGB.to_vec();
    for o in [8, 12, 24, 28] {
        huge[o..o + 4].copy_from_slice(&(1u32 << 20).to_be_bytes());
    }
    expect(t, "JPX too large", jpx("/Width 8 /Height 8 /ColorSpace /DeviceRGB /Filter /JPXDecode", &huge), &[(50.0, 50.0, WHITE)], 0);
}

// JPEG 2000 images for render_image, made by `python tools/jp2_check.py --fixtures` (Pillow with
// OpenJPEG 2.5): 8 x 8 (the lossy one 16 x 16), the top half (40, 160, 200), the bottom half
// (200, 40, 90), grey 60 and 220; the RGBA one has opacity 0 in its bottom half; J2K_GRAY4 has
// 4-bit samples, 1 in its top half and 5 in its bottom half

// ------------------------------------------------------------------ render_shade

fn render_shade(t: &mut T) {
    let rb = "<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>";
    let shd = |s: String| format!("/Shading << /S {s} >>");
    expect(t, "axial", page(b"/S sh", &shd(format!("<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function {rb} >>")), &[], ""), &[(0.5, 50.0, [254, 0, 1]), (99.5, 50.0, [1, 0, 254]), (50.0, 50.0, [127, 0, 128])], 2);
    expect(t, "axial without Extend", page(b"/S sh", &shd(format!("<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [25 0 75 0] /Function {rb} >>")), &[], ""), &[(10.0, 50.0, WHITE), (90.0, 50.0, WHITE), (30.0, 50.0, [227, 0, 28])], 3);
    expect(t, "axial with Extend", page(b"/S sh", &shd(format!("<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [25 0 75 0] /Function {rb} /Extend [true true] >>")), &[], ""), &[(10.0, 50.0, RED), (90.0, 50.0, BLUE)], 1);
    expect(t, "radial", page(b"/S sh", &shd(format!("<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [50 50 0 50 50 40] /Function {rb} >>")), &[], ""), &[(50.0, 50.0, [250, 0, 5]), (50.0, 89.0, [9, 0, 246]), (5.0, 5.0, WHITE)], 4);
    expect(t, "radial with Extend", page(b"/S sh", &shd(format!("<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [50 50 0 50 50 40] /Function {rb} /Extend [false true] >>")), &[], ""), &[(5.0, 5.0, BLUE)], 1);
    let f4 = (10, stream("/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1]", b"{ 0 }"));
    expect(t, "function-based", page(b"/S sh", &shd(String::from("<< /ShadingType 1 /ColorSpace /DeviceRGB /Matrix [100 0 0 100 0 0] /Function 10 0 R >>")), &[f4], ""), &[(5.0, 95.0, [14, 241, 0]), (95.0, 5.0, [244, 11, 0])], 3);
    let mut mesh = Vec::new();
    for (x, y, c) in [(0u16, 0u16, [255u8, 0, 0]), (100, 0, [0, 255, 0]), (0, 100, [0, 0, 255])] {
        mesh.push(0);
        mesh.extend_from_slice(&x.to_be_bytes());
        mesh.extend_from_slice(&y.to_be_bytes());
        mesh.extend_from_slice(&c);
    }
    let m4 = (10, stream("/ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 16 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 65535 0 65535 0 1 0 1 0 1]", &mesh));
    expect(t, "triangle mesh", page(b"/S sh", "/Shading << /S 10 0 R >>", &[m4], ""), &[(1.0, 1.0, [252, 2, 2]), (98.0, 1.0, [3, 251, 2]), (1.0, 98.0, [2, 3, 251]), (80.0, 80.0, WHITE)], 6);
    // a Coons patch that is a square with four colours at its corners
    let mut coons = vec![0u8];
    for (x, y) in [(0u8, 0u8), (0, 33), (0, 67), (0, 100), (33, 100), (67, 100), (100, 100), (100, 67), (100, 33), (100, 0), (67, 0), (33, 0)] {
        coons.extend_from_slice(&[x, y]);
    }
    coons.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]);
    let m6 = (10, stream("/ShadingType 6 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 255 0 255 0 1 0 1 0 1]", &coons));
    expect(t, "Coons patch", page(b"/S sh", "/Shading << /S 10 0 R >>", &[m6], ""), &[(1.0, 1.0, [250, 2, 0]), (1.0, 98.0, [2, 250, 0]), (98.0, 98.0, [2, 2, 250]), (98.0, 1.0, [250, 250, 2])], 8);
    // a shading pattern with a background, filling a path
    let pat = format!("/Pattern << /P << /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [40 0 60 0] /Function {rb} /Background [0 1 0] >> >> >>");
    expect(t, "shading pattern", page(b"/Pattern cs /P scn 10 10 80 80 re f", &pat, &[], ""), &[(50.0, 50.0, [121, 0, 134]), (20.0, 50.0, GREEN), (5.0, 5.0, WHITE)], 3);
    // tiling patterns: a coloured one, an uncoloured one in two colours
    let tile = (10, stream("/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 20 20] /XStep 20 /YStep 20 /Resources << >>", b"1 0 0 rg 0 0 10 10 re f 0 0 1 rg 10 10 10 10 re f"));
    expect(t, "tiling pattern", page(b"/Pattern cs /P scn 0 0 100 60 re f", "/Pattern << /P 10 0 R >>", &[tile], ""), &[(5.0, 5.0, RED), (15.0, 15.0, BLUE), (15.0, 5.0, WHITE), (45.0, 25.0, RED), (55.0, 55.0, BLUE), (55.0, 75.0, WHITE)], 0);
    let unc = (10, stream("/PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 20 20] /XStep 20 /YStep 20 /Resources << >>", b"1 0 0 rg 0 0 10 20 re f"));
    expect(
        t,
        "uncoloured pattern",
        page(b"/C cs 0 1 0 /P scn 0 0 50 100 re f /C cs 0 0 1 /P scn 50 0 50 100 re f", "/Pattern << /P 10 0 R >> /ColorSpace << /C [/Pattern /DeviceRGB] >>", &[unc], ""),
        &[(5.0, 50.0, GREEN), (15.0, 50.0, WHITE), (65.0, 50.0, BLUE)],
        0,
    );
}

// ------------------------------------------------------------------ render_transp

fn render_transp(t: &mut T) {
    let gs = "/ExtGState << /A << /ca 0.5 /CA 0.5 >> /M << /BM /Multiply >> /S << /BM /Screen >> /D << /BM /Difference >> /L << /BM /Luminosity >> \
              /K << /SMask << /S /Luminosity /G 10 0 R >> >> /J << /SMask << /S /Alpha /G 11 0 R >> >> /N << /SMask /None >> >> /XObject << /G 12 0 R >>";
    let extra = vec![
        (10, stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceGray >>", b"0.5 g 0 0 50 100 re f 1 g 50 0 50 100 re f")),
        (11, stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >> /Resources << /ExtGState << /H << /ca 0.25 >> >> >>", b"/H gs 0 0 50 100 re f")),
        (12, stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >>", b"1 0 0 rg 0 0 60 100 re f 0 0 1 rg 40 0 60 100 re f")),
    ];
    let p = |c: &[u8]| page(c, gs, &extra, "");
    expect(t, "fill alpha", p(b"/A gs 1 0 0 rg 0 0 100 100 re f"), &[(50.0, 50.0, [255, 128, 128])], 1);
    expect(t, "stroke alpha", p(b"/A gs 0 g 20 w 0 50 m 100 50 l S"), &[(50.0, 50.0, [128, 128, 128])], 1);
    expect(t, "alpha ends with Q", p(b"q /A gs Q 1 0 0 rg 0 0 100 100 re f"), &[(50.0, 50.0, RED)], 0);
    expect(t, "Multiply", p(b"0 0 1 rg 0 0 100 100 re f /M gs 1 1 0 rg 0 0 100 100 re f"), &[(50.0, 50.0, BLACK)], 1);
    expect(t, "Multiply with grey", p(b"0.5 g 0 0 100 100 re f /M gs 0.5 g 0 0 100 100 re f"), &[(50.0, 50.0, [64, 64, 64])], 1);
    expect(t, "Screen", p(b"0.5 g 0 0 100 100 re f /S gs 0.5 g 0 0 100 100 re f"), &[(50.0, 50.0, [191, 191, 191])], 1);
    expect(t, "Difference", p(b"1 0 0 rg 0 0 100 100 re f /D gs 1 1 1 rg 0 0 100 100 re f"), &[(50.0, 50.0, [0, 255, 255])], 1);
    expect(t, "Luminosity", p(b"1 0 0 rg 0 0 100 100 re f /L gs 0.5 g 0 0 100 100 re f"), &[(50.0, 50.0, [255, 73, 73])], 2);
    // over the paper a blend mode sees nothing: the colour comes out as it is
    expect(t, "Screen over the paper", p(b"/S gs 1 0 0 rg 0 0 100 100 re f"), &[(50.0, 50.0, RED)], 0);
    expect(t, "luminosity soft mask", p(b"/K gs 0 g 0 0 100 100 re f"), &[(25.0, 50.0, [128, 128, 128]), (75.0, 50.0, BLACK)], 2);
    expect(t, "alpha soft mask", p(b"/J gs 0 g 0 0 100 100 re f"), &[(25.0, 50.0, [191, 191, 191]), (75.0, 50.0, WHITE)], 2);
    expect(t, "soft mask /None", p(b"/K gs /N gs 0 g 0 0 100 100 re f"), &[(25.0, 50.0, BLACK)], 0);
    // a group at half alpha: where its two rectangles overlap, only the top one shows
    expect(t, "transparency group", p(b"/A gs /G Do"), &[(20.0, 50.0, [255, 128, 128]), (50.0, 50.0, [128, 128, 255]), (80.0, 50.0, [128, 128, 255])], 1);
}

// ------------------------------------------------------------------ render_text

/// eexec / charstring encryption: r, then n zero bytes before the data
fn encrypt(data: &[u8], r0: u16, n: usize) -> Vec<u8> {
    let mut r = r0;
    let mut out = Vec::new();
    for &p in core::iter::repeat_n(&0u8, n).chain(data.iter()) {
        let c = p ^ (r >> 8) as u8;
        r = (c as u16).wrapping_add(r).wrapping_mul(52845).wrapping_add(22719);
        out.push(c);
    }
    out
}

/// a Type 1 charstring: numbers, and operators as -1_000_000 - op (12 xx as 1200 + xx)
fn charstring(ops: &[i32]) -> Vec<u8> {
    let mut out = Vec::new();
    for &v in ops {
        if v < -100_000 {
            let op = -(v + 1_000_000);
            if op >= 1200 {
                out.extend_from_slice(&[12, (op - 1200) as u8]);
            } else {
                out.push(op as u8);
            }
        } else {
            num(&mut out, v);
        }
    }
    encrypt(&out, 4330, 4)
}

fn num(out: &mut Vec<u8>, v: i32) {
    match v {
        -107..=107 => out.push((v + 139) as u8),
        108..=1131 => {
            let w = v - 108;
            out.extend_from_slice(&[(247 + (w >> 8)) as u8, w as u8]);
        }
        -1131..=-108 => {
            let w = -v - 108;
            out.extend_from_slice(&[(251 + (w >> 8)) as u8, w as u8]);
        }
        _ => {
            out.push(255);
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
}

const HSBW: i32 = -1_000_013;
const RMOVETO: i32 = -1_000_021;
const RLINETO: i32 = -1_000_005;
const CLOSEPATH: i32 = -1_000_009;
const ENDCHAR: i32 = -1_000_014;
const CALLSUBR: i32 = -1_000_010;
const RETURN: i32 = -1_000_011;
const SEAC: i32 = -1_001_206;

/// a Type 1 font: A a square of 800, B a square of 200, C a triangle through a subroutine,
/// Aacute A with B above it by seac
fn type1_font() -> Vec<u8> {
    let rd = |name: &str, cs: Vec<u8>| -> Vec<u8> { cat(&[format!("/{name} {} RD ", cs.len()).as_bytes(), &cs, b" ND\n"]) };
    let sq = |s: i32, w: i32| charstring(&[0, w, HSBW, 0, 0, RMOVETO, s, 0, RLINETO, 0, s, RLINETO, -s, 0, RLINETO, CLOSEPATH, ENDCHAR]);
    let subr = charstring(&[0, 0, RMOVETO, 500, 0, RLINETO, 0, 500, RLINETO, CLOSEPATH, RETURN]);
    let private = cat(&[
        b"dup /Private 8 dict dup begin\n/RD {string currentfile exch readstring pop} executeonly def\n/ND {noaccess def} executeonly def\n/NP {noaccess put} executeonly def\n/lenIV 4 def\n/Subrs 1 array\n",
        format!("dup 0 {} RD ", subr.len()).as_bytes(),
        &subr,
        b" NP\nND\n2 index /CharStrings 5 dict dup begin\n",
        &rd(".notdef", charstring(&[0, 500, HSBW, ENDCHAR])),
        &rd("A", sq(800, 1000)),
        &rd("B", sq(200, 300)),
        &rd("C", charstring(&[0, 1000, HSBW, 0, CALLSUBR, ENDCHAR])),
        &rd("Aacute", charstring(&[0, 1000, HSBW, 0, 300, 900, 65, 66, SEAC])),
        b"end\nend\nmark currentfile closefile\n",
    ]);
    let clear = b"%!PS-AdobeFont-1.0: Test 1\n11 dict begin\n/FontName /Test def\n/FontType 1 def\n/PaintType 0 def\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/Encoding StandardEncoding def\n/FontBBox {0 0 1000 1000} readonly def\ncurrentdict end\ncurrentfile eexec\n";
    let enc = encrypt(&private, 55665, 4);
    let tail = cat(&["0".repeat(512).as_bytes(), b"\ncleartomark\n"]);
    let data = cat(&[clear, &enc, &tail]);
    stream(&format!("/Length1 {} /Length2 {} /Length3 {}", clear.len(), enc.len(), tail.len()), &data)
}

fn render_text(t: &mut T) {
    // a Type 3 font: A a square, B a triangle in its own colour (d0)
    let t3 = "/Font << /F3 << /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /sq 10 0 R /tri 11 0 R >> \
              /Encoding << /Differences [65 /sq /tri] >> /FirstChar 65 /LastChar 66 /Widths [1000 1000] >> >>";
    let procs = vec![(10, stream("", b"1000 0 0 0 1000 1000 d1 100 100 800 800 re f")), (11, stream("", b"1000 0 d0 1 0 0 rg 100 100 m 900 100 l 500 900 l h f"))];
    let p = |c: &[u8]| page(c, t3, &procs, "");
    expect(t, "Type 3 glyph in the fill colour", p(b"0 0 1 rg BT /F3 40 Tf 10 10 Td (A) Tj ET"), &[(30.0, 30.0, BLUE), (12.0, 12.0, WHITE)], 0);
    expect(t, "Type 3 glyph in its own colour", p(b"0 0 1 rg BT /F3 40 Tf 10 10 Td (B) Tj ET"), &[(30.0, 20.0, RED)], 0);
    expect(t, "Type 3 advance", p(b"0 g BT /F3 40 Tf 10 10 Td (AA) Tj ET"), &[(30.0, 30.0, BLACK), (70.0, 30.0, BLACK), (52.0, 30.0, WHITE)], 0);
    expect(t, "character spacing", p(b"0 g BT /F3 20 Tf 10 Tc 10 10 Td (AA) Tj ET"), &[(20.0, 20.0, BLACK), (50.0, 20.0, BLACK), (40.0, 20.0, WHITE)], 0);
    expect(t, "TJ adjustment", p(b"0 g BT /F3 20 Tf 10 10 Td [(A) -1000 (A)] TJ ET"), &[(20.0, 20.0, BLACK), (60.0, 20.0, BLACK), (40.0, 20.0, WHITE)], 0);
    expect(t, "horizontal scaling", p(b"0 g BT /F3 20 Tf 200 Tz 10 10 Td (A) Tj ET"), &[(45.0, 20.0, BLACK), (55.0, 20.0, WHITE)], 0);
    expect(t, "text rise", p(b"0 g BT /F3 20 Tf 20 Ts 10 10 Td (A) Tj ET"), &[(20.0, 40.0, BLACK), (20.0, 20.0, WHITE)], 0);
    expect(t, "turned text", p(b"0 g BT /F3 40 Tf 0 1 -1 0 90 10 Tm (A) Tj ET"), &[(70.0, 30.0, BLACK), (30.0, 30.0, WHITE)], 0);
    expect(t, "invisible text", p(b"0 g BT 3 Tr /F3 40 Tf 10 10 Td (A) Tj ET"), &[(30.0, 30.0, WHITE)], 0);
    // Type 3 glyphs do not clip (as in Poppler)
    expect(t, "Type 3 text does not clip", p(b"BT 7 Tr /F3 40 Tf 10 10 Td (A) Tj ET 0 1 0 rg 0 0 100 100 re f"), &[(30.0, 30.0, GREEN), (80.0, 80.0, GREEN)], 0);
    expect(t, "clip text that draws nothing", p(b"BT 7 Tr ET 0 1 0 rg 0 0 100 100 re f"), &[(50.0, 50.0, WHITE)], 0);
    // a Type 1 font: squares, a subroutine, seac
    let widths = (65..=200).map(|c| if c == 66 { "300" } else { "1000" }).collect::<Vec<_>>().join(" ");
    let t1 = format!(
        "/Font << /T1 << /Type /Font /Subtype /Type1 /BaseFont /Test /FirstChar 65 /LastChar 200 /Widths [{widths}] /Encoding << /Differences [200 /Aacute] >> /FontDescriptor 10 0 R >> >>"
    );
    let objs = vec![
        (10, String::from("<< /Type /FontDescriptor /FontName /Test /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 1000 /Descent 0 /CapHeight 800 /StemV 50 /FontFile 11 0 R >>").into_bytes()),
        (11, type1_font()),
    ];
    expect(
        t,
        "Type 1 font",
        page(b"0 g BT /T1 40 Tf 10 10 Td (A) Tj 45 0 Td (C) Tj ET BT /T1 40 Tf 10 50 Td <C8> Tj ET", &t1, &objs, ""),
        &[(26.0, 26.0, BLACK), (45.0, 26.0, WHITE), (72.0, 13.0, BLACK), (58.0, 27.0, WHITE), (26.0, 70.0, BLACK), (26.0, 90.0, BLACK), (26.0, 84.0, WHITE), (15.0, 90.0, WHITE)],
        0,
    );
    expect(t, "clipping text", page(b"BT 7 Tr /T1 40 Tf 10 10 Td (A) Tj ET 0 1 0 rg 0 0 100 100 re f", &t1, &objs, ""), &[(26.0, 26.0, GREEN), (80.0, 80.0, WHITE)], 0);
    expect(t, "filling and clipping text", page(b"0 0 1 rg BT 4 Tr /T1 40 Tf 10 10 Td (A) Tj ET 0 1 0 rg 0 0 100 30 re f", &t1, &objs, ""), &[(26.0, 20.0, GREEN), (26.0, 38.0, BLUE), (80.0, 20.0, WHITE)], 0);
    // stroked text in a Type 1 font: the outline, not the inside
    expect(t, "stroked glyph", page(b"0 g 2 w BT 1 Tr /T1 80 Tf 10 10 Td (A) Tj ET", &t1, &objs, ""), &[(42.0, 42.0, WHITE), (10.0, 40.0, BLACK)], 0);
    // the writer's sample: every glyph of its embedded fonts leaves ink where page_text says
    let Ok(sample) = crate::sample::build() else {
        t.check("the sample builds", false);
        return;
    };
    let Some(doc) = t.ok("the sample opens", Document::open(sample).map_err(|e| format!("{e}"))) else { return };
    let (mut with_ink, mut glyphs) = (0, 0);
    for pg in 0..doc.page_count() {
        let (Ok(text), Ok(sf)) = (doc.page_text(pg), doc.render(pg, &RenderOptions::default())) else {
            t.check(&format!("sample page {pg} draws"), false);
            continue;
        };
        let px = Px::of(&sf);
        let Ok(info) = doc.page(pg) else { continue };
        let top = info.crop_box[3];
        for it in &text.items {
            // letters and digits: they have ink in the middle of their line
            if !it.text.chars().all(char::is_alphanumeric) || it.text.is_empty() || it.size < 6.0 || it.dy.abs() > 0.01 || it.width < 2.0 {
                continue;
            }
            glyphs += 1;
            let (x0, x1) = (it.x + it.width * 0.15, it.x + it.width * 0.85);
            let (y0, y1) = (it.y + it.size * 0.1, it.y + it.size * 0.6);
            let mut ink = false;
            let mut y = y0;
            while y < y1 && !ink {
                let mut x = x0;
                while x < x1 {
                    let (dx, dy) = (x as usize, (top - y) as usize);
                    if dx < px.w && dy < px.h && px.dev(dx, dy).iter().any(|&c| c < 200) {
                        ink = true;
                        break;
                    }
                    x += 0.5;
                }
                y += 0.5;
            }
            with_ink += ink as u32;
        }
    }
    t.check(&format!("the sample's glyphs leave ink: {with_ink} of {glyphs}"), glyphs > 100 && with_ink * 100 >= glyphs * 97);
}

// ------------------------------------------------------------------ render_annot

fn render_annot(t: &mut T) {
    let ap = stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", b"0 0 1 rg 0 0 10 10 re f");
    let on = stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", b"0 1 0 rg 0 0 10 10 re f");
    let objs = vec![
        (10, ap),
        (11, String::from("<< /Type /Annot /Subtype /Square /Rect [10 10 40 40] /AP << /N 10 0 R >> >>").into_bytes()),
        (12, String::from("<< /Type /Annot /Subtype /Square /Rect [60 10 90 40] /F 2 /AP << /N 10 0 R >> >>").into_bytes()),
        (13, String::from("<< /Type /Annot /Subtype /Widget /Rect [10 60 40 90] /AS /On /AP << /N << /On 14 0 R /Off 10 0 R >> >> >>").into_bytes()),
        (14, on),
        (15, String::from("<< /Type /Annot /Subtype /Square /Rect [60 60 90 90] /F 32 /AP << /N 10 0 R >> >>").into_bytes()),
    ];
    let data = page(b"", "", &objs, "/Annots [11 0 R 12 0 R 13 0 R 15 0 R]");
    expect(t, "annotations", data.clone(), &[(25.0, 25.0, BLUE), (75.0, 25.0, WHITE), (25.0, 75.0, GREEN), (75.0, 75.0, WHITE), (5.0, 5.0, WHITE)], 0);
    if let Some(px) = draw_opt(t, "annotations left out", data, &RenderOptions { annotations: false, ..Default::default() }) {
        t.eq("annotations left out: none drawn", px.at(25.0, 25.0), WHITE);
    }
    // optional content: an OCG that is off, a membership dictionary, an XObject in a group
    let mut m = Mk::new();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [10 0 R 11 0 R] /D << /OFF [11 0 R] >> >> >>")
        .add(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .add(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Properties << /on 10 0 R /off 11 0 R /all 12 0 R /any 13 0 R >> /XObject << /X 14 0 R >> >> >>")
        .add(
            4,
            flate("", b"/OC /on BDC 1 0 0 rg 0 0 50 50 re f EMC /OC /off BDC 0 0 1 rg 50 0 50 50 re f EMC /OC /all BDC 0 1 0 rg 0 50 50 50 re f EMC /OC /any BDC 0 0 0 rg 50 50 50 50 re f EMC /X Do"),
        )
        .add(10, "<< /Type /OCG /Name (an) >>")
        .add(11, "<< /Type /OCG /Name (aus) >>")
        .add(12, "<< /Type /OCMD /OCGs [10 0 R 11 0 R] /P /AllOn >>")
        .add(13, "<< /Type /OCMD /OCGs [10 0 R 11 0 R] /P /AnyOn >>")
        .add(14, stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /OC 11 0 R", b"1 1 0 rg 0 0 100 100 re f"));
    expect(t, "optional content", m.table("/Root 1 0 R"), &[(25.0, 25.0, RED), (75.0, 25.0, WHITE), (25.0, 75.0, WHITE), (75.0, 75.0, BLACK)], 0);
}

// ------------------------------------------------------------------ render_hostile

fn render_all(data: Vec<u8>, dpi: f64) -> bool {
    let Ok(doc) = Document::open(data) else { return false };
    let opt = RenderOptions { dpi, ..Default::default() };
    for i in 0..doc.page_count().min(4) {
        let _ = doc.render(i, &opt);
    }
    true
}

fn render_hostile(t: &mut T) {
    // a page too large to draw
    let mut m = Mk::new();
    m.add(1, "<< /Type /Catalog /Pages 2 0 R >>").add(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>").add(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1000000000 1000000000] >>");
    if let Some(doc) = t.ok("huge page opens", Document::open(m.table("/Root 1 0 R")).map_err(|e| format!("{e}"))) {
        t.check("a huge page is a limit", matches!(doc.render(0, &RenderOptions::default()), Err(Error::Limit(_))));
    }
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    // forms drawing each other 16 times, ten deep
    let mut objs = Vec::new();
    for k in 0..10u32 {
        let body = if k == 9 { String::from("0 0 100 100 re f") } else { "/N Do ".repeat(16) };
        objs.push((20 + k, stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /N {} 0 R >> >>", 21 + k), body.as_bytes())));
    }
    cases.push(("forms that draw each other", page(b"/N Do", "/XObject << /N 20 0 R >>", &objs, "")));
    // a form that draws itself, a pattern that paints with itself, a glyph that shows itself, a
    // soft mask whose group sets it again
    cases.push(("a form drawing itself", page(b"/N Do", "/XObject << /N 10 0 R >>", &[(10, stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /N 10 0 R >> >>", b"/N Do /N Do"))], "")));
    cases.push((
        "a pattern painting with itself",
        page(b"/Pattern cs /P scn 0 0 100 100 re f", "/Pattern << /P 10 0 R >>", &[(10, stream("/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 5 5] /XStep 5 /YStep 5 /Resources << /Pattern << /P 10 0 R >> >>", b"/Pattern cs /P scn 0 0 5 5 re f"))], ""),
    ));
    cases.push((
        "a glyph showing itself",
        page(
            b"BT /F 20 Tf 10 10 Td (AAAA) Tj ET",
            "/Font << /F 10 0 R >>",
            &[
                (10, String::from("<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.01 0 0 0.01 0 0] /CharProcs << /a 11 0 R >> /Encoding << /Differences [65 /a] >> /FirstChar 65 /LastChar 65 /Widths [100] /Resources << /Font << /F 10 0 R >> >> >>").into_bytes()),
                (11, stream("", b"100 0 d0 BT /F 100 Tf (AA) Tj ET 0 0 50 50 re f")),
            ],
            "",
        ),
    ));
    cases.push((
        "a soft mask setting itself",
        page(
            b"/S gs 0 0 100 100 re f",
            "/ExtGState << /S << /SMask << /S /Luminosity /G 10 0 R >> >> >>",
            &[(10, stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >> /Resources << /ExtGState << /S << /SMask << /S /Luminosity /G 10 0 R >> >> >> >>", b"/S gs 1 g 0 0 100 100 re f"))],
            "",
        ),
    ));
    // many large fills: the work of a page is counted
    cases.push(("200 000 fills of the page", page(&b"0 0 100 100 re f ".repeat(200_000), "", &[], "")));
    // a mesh of many triangles over the page
    let mut mesh = Vec::new();
    for i in 0..30_000u32 {
        for (x, y) in [(0u16, 0u16), (65535, (i % 65535) as u16), (0, 65535)] {
            mesh.push(0);
            mesh.extend_from_slice(&x.to_be_bytes());
            mesh.extend_from_slice(&y.to_be_bytes());
            mesh.extend_from_slice(&[200, 0, 0]);
        }
    }
    cases.push(("a large mesh", page(b"/S sh", "/Shading << /S 10 0 R >>", &[(10, flate("/ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 16 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 100 0 100 0 1 0 1 0 1]", &mesh))], "")));
    // images: huge sizes, short data, broken JPEG and CCITT data
    let xo = "/XObject << /I 10 0 R >>";
    for (name, dict, data) in [
        ("a huge image", "/Width 1000000 /Height 1000000 /ColorSpace /DeviceRGB /BitsPerComponent 8", &b"abc"[..]),
        ("an image without its data", "/Width 50 /Height 50 /ColorSpace /DeviceRGB /BitsPerComponent 8", &b""[..]),
        ("a broken JPEG", "/Width 50 /Height 50 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", &b"\xFF\xD8\xFF\xC0garbage"[..]),
        ("broken CCITT data", "/Width 50 /Height 50 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K 1 /Columns 50 >>", &b"\x00\x01\xFF\x7F\x80"[..]),
        ("an odd bit depth", "/Width 5 /Height 5 /ColorSpace /DeviceRGB /BitsPerComponent 7", &b"xxxxxxxxxxxxxxxxxxxx"[..]),
    ] {
        cases.push((name, page(b"q 100 0 0 100 0 0 cm /I Do Q", xo, &[(10, img(dict, data))], "")));
    }
    // broken font programs
    for (name, key, dict, data) in [
        ("a broken Type 1 program", "FontFile", "/Length1 10 /Length2 10 /Length3 0", &b"%!PS eexec \x00\x01\x02\x03 /CharStrings 1 dict dup /A 999999 RD"[..]),
        ("a broken CFF", "FontFile3", "/Subtype /Type1C", &b"\x01\x00\x04\x04\x00\x01\x01\x01\xFF\xFF"[..]),
        ("a broken TrueType font", "FontFile2", "", &b"\x00\x01\x00\x00\x00\x09\x00\x80head"[..]),
    ] {
        cases.push((
            name,
            page(
                b"BT /F 20 Tf 10 10 Td (Hallo) Tj ET",
                "/Font << /F 10 0 R >>",
                &[
                    (10, String::from("<< /Type /Font /Subtype /Type1 /BaseFont /Broken /FirstChar 32 /LastChar 126 /Widths [500] /FontDescriptor 11 0 R >>").into_bytes()),
                    (11, format!("<< /Type /FontDescriptor /FontName /Broken /Flags 32 /{key} 12 0 R >>").into_bytes()),
                    (12, stream(dict, data)),
                ],
                "",
            ),
        ));
    }
    // functions: a deep PostScript program, a sampled function too large
    cases.push((
        "odd functions",
        page(
            b"/A cs 0.5 scn 0 0 50 100 re f /B cs 0.5 scn 50 0 50 100 re f",
            "/ColorSpace << /A [/Separation /a /DeviceRGB 10 0 R] /B [/Separation /b /DeviceRGB 11 0 R] >>",
            &[
                (10, stream("/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]", &cat(&[&b"{ ".repeat(500), &b"} ".repeat(500)]))),
                (11, stream("/FunctionType 0 /Domain [0 1] /Range [0 1 0 1 0 1] /Size [99999999] /BitsPerSample 32", b"x")),
            ],
            "",
        ),
    ));
    for (name, data) in cases {
        render_all(data, 72.0);
        t.check(&format!("{name}: drawn through"), true);
    }
    // mutations of pages that draw much, with a fixed seed
    let mut seeds: Vec<Vec<u8>> = Vec::new();
    if let Ok(s) = crate::sample::build() {
        seeds.push(s);
    }
    seeds.push(page(b"q 1 0 0 rg 0 0 50 50 re f 2 w 0 g 10 10 m 90 90 l S Q /S sh q 80 0 0 80 10 10 cm /I Do Q BT /F3 20 Tf 10 10 Td (AB) Tj ET", "/Shading << /S << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >> /XObject << /I 10 0 R >> /Font << /F3 << /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 11 0 R >> /Encoding << /Differences [65 /a /a] >> /FirstChar 65 /LastChar 66 /Widths [1000 1000] >> >>", &[(10, img("/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 12 0 R", &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255])), (11, stream("", b"1000 0 d0 0 0 1000 1000 re f")), (12, img("/Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", &[0, 80, 160, 255]))], ""));
    let tokens: &[&[u8]] = &[b"obj", b"<<", b">>", b"[", b"]", b"(", b")", b"re f", b" 0 R", b"/Length", b"\x00", b"999999999999", b"-1", b"q", b"Q", b"cm", b"Do", b"sh", b"BT"];
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = move |n: usize| -> usize {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % n.max(1) as u64) as usize
    };
    let (mut drawn, mut total) = (0, 0);
    for round in 0..300 {
        let mut d = seeds[round % seeds.len()].clone();
        for _ in 0..1 + rnd(6) {
            if d.is_empty() {
                break;
            }
            let at = rnd(d.len());
            match rnd(4) {
                0 => d[at] = rnd(256) as u8,
                1 => d[at] ^= 1 << rnd(8),
                2 => {
                    let tk = tokens[rnd(tokens.len())];
                    d.splice(at..at, tk.iter().copied());
                }
                _ => {
                    let len = rnd(64).min(d.len() - at);
                    let piece = d[at..at + len].to_vec();
                    let to = rnd(d.len());
                    d.splice(to..to, piece);
                }
            }
        }
        total += 1;
        if render_all(d, 36.0) {
            drawn += 1;
        }
    }
    t.check(&format!("{total} mutated files drawn without a panic ({drawn} opened)"), total == 300 && drawn > 75);
}
const J2K_RGB: &[u8] = &[0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5D, 0x00, 0x01, 0xFF, 0x93, 0xCF, 0xB4, 0x3C, 0x10, 0xCC, 0x4E, 0x0C, 0x04, 0xEC, 0x3D, 0x61, 0xE6, 0x13, 0x9C, 0x5C, 0x52, 0x33, 0x0F, 0xCF, 0xB4, 0x38, 0x06, 0x31, 0x3A, 0xE1, 0xF6, 0xBC, 0x75, 0x91, 0x87, 0x01, 0xE9, 0x44, 0x92, 0x7F, 0xCF, 0xB4, 0x38, 0x13, 0x8C, 0x45, 0x30, 0x14, 0x4D, 0x30, 0x11, 0x36, 0x09, 0x0C, 0x00, 0x02, 0x9F, 0xA3, 0xED, 0x05, 0x00, 0x0C, 0x78, 0x3B, 0x2E, 0x9F, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0x97, 0x3C, 0x5B, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0xA7, 0x14, 0x7F, 0xFF, 0xD9];  // 208 bytes
const JP2_RGB: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x2D, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x03, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0xD8, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5D, 0x00, 0x01, 0xFF, 0x93, 0xCF, 0xB4, 0x3C, 0x10, 0xCC, 0x4E, 0x0C, 0x04, 0xEC, 0x3D, 0x61, 0xE6, 0x13, 0x9C, 0x5C, 0x52, 0x33, 0x0F, 0xCF, 0xB4, 0x38, 0x06, 0x31, 0x3A, 0xE1, 0xF6, 0xBC, 0x75, 0x91, 0x87, 0x01, 0xE9, 0x44, 0x92, 0x7F, 0xCF, 0xB4, 0x38, 0x13, 0x8C, 0x45, 0x30, 0x14, 0x4D, 0x30, 0x11, 0x36, 0x09, 0x0C, 0x00, 0x02, 0x9F, 0xA3, 0xED, 0x05, 0x00, 0x0C, 0x78, 0x3B, 0x2E, 0x9F, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0x97, 0x3C, 0x5B, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0xA7, 0x14, 0x7F, 0xFF, 0xD9];  // 293 bytes
const JP2_GRAY: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x2D, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x9B, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x29, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x26, 0x00, 0x01, 0xFF, 0x93, 0xCF, 0xB4, 0x30, 0x11, 0x11, 0xFA, 0xAE, 0xD9, 0xCA, 0xF1, 0xBB, 0xB6, 0xA8, 0xE2, 0xC1, 0xA3, 0xED, 0x05, 0x00, 0x0C, 0x78, 0x3B, 0x2E, 0x9F, 0xFF, 0xD9];  // 232 bytes
const JP2_RGBA: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x4F, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x04, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x22, 0x63, 0x64, 0x65, 0x66, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF5, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x77, 0x00, 0x01, 0xFF, 0x93, 0xCF, 0xB4, 0x3C, 0x10, 0xCC, 0x4E, 0x0C, 0x04, 0xEC, 0x3D, 0x61, 0xE6, 0x13, 0x9C, 0x5C, 0x52, 0x33, 0x0F, 0xCF, 0xB4, 0x38, 0x06, 0x31, 0x3A, 0xE1, 0xF6, 0xBC, 0x75, 0x91, 0x87, 0x01, 0xE9, 0x44, 0x92, 0x7F, 0xCF, 0xB4, 0x38, 0x13, 0x8C, 0x45, 0x30, 0x14, 0x4D, 0x30, 0x11, 0x36, 0x09, 0x0C, 0x00, 0x02, 0x9F, 0xDF, 0x80, 0x88, 0x0E, 0xCA, 0x2E, 0x01, 0xF9, 0x74, 0xBC, 0x2C, 0xF8, 0x7C, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xA3, 0xED, 0x05, 0x00, 0x0C, 0x78, 0x3B, 0x2E, 0x9F, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0x97, 0x3C, 0x5B, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0xA7, 0x14, 0x7F, 0xA7, 0xE0, 0x06, 0x0F, 0x28, 0x5F, 0xFF, 0xD9];  // 356 bytes
const JP2_LOSSY: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x2D, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x00, 0x03, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0xAF, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x04, 0x04, 0x00, 0x00, 0xFF, 0x5C, 0x00, 0x11, 0x42, 0x5F, 0x52, 0x50, 0x05, 0x50, 0x05, 0x50, 0x47, 0x57, 0xD3, 0x57, 0xD3, 0x57, 0x62, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x00, 0x01, 0xFF, 0x93, 0xC4, 0xA0, 0x10, 0xCC, 0x4E, 0x0A, 0xDF, 0xC7, 0x85, 0x00, 0xC8, 0xAA, 0x22, 0xB2, 0xC7, 0x46, 0x13, 0x8C, 0x45, 0x30, 0x28, 0x2C, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0xFF, 0xD9];  // 252 bytes
const J2K_GRAY4: &[u8] = &[0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x29, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1B, 0x00, 0x01, 0xFF, 0x93, 0xC0, 0xF8, 0x44, 0x10, 0xDC, 0x38, 0x3D, 0xA0, 0x1D, 0x0C, 0x0C, 0x78, 0x3F, 0xFF, 0xD9];  // 136 bytes
