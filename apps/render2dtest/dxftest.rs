// dxftest -- G6e2: DXF drawings in render2d (lib/dxf reads, render2d::dxf maps; docs/RENDER2D.md "DXF").
//
//   render2dtest --test dxf              the reader and the mapping on drawings written here (FIXTURE and friends: every
//                                        entity kind, tables, blocks, code pages, line ends, broken and crafted files,
//                                        limits, pieces of one byte), the GPU against the CPU path on them
//   render2dtest --test dxfcorpus        every file of the corpus outside the repository ($JFLIB_DXF_CORPUS, else
//                                        Documents/JFLib-dxf-korpus in the home folder; SKIP without it) but Zürich: no
//                                        error but for binary and fuzzer files, the kinds read against an independent
//                                        count of the file's entity markers, the GPU against the CPU path; R2D_DXF_PNG=DIR
//                                        writes the CPU path's images as PNG
//   render2dtest --dxf PATH [--no-gpu]   the demo's view on a drawing (pan, zoom, pick, snap; a DXF entity as a whole)
//   render2dtest --dxf-shot PATH OUT.png [--res WxH] [--gpu]   a drawing whole, through the CPU path (or the GPU)
//   render2dtest --bench-dxf PATH [--res WxH] [--frames F]     read, insert, GPU and CPU frames of a large drawing, memory

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::dxf::{Family, Loaded, Options};
use render2d::{Camera, Renderer, RendererDesc, Scene};

use crate::gallery::*;
use crate::refimg;
use crate::tests::{out, setup, shot, T};

// ---------------------------------------------------------------------------------------------------- fonts

static SANS_BOLD: &[u8] = include_bytes!("../../styles/netlab/assets/Inter-SemiBold.ttf");
static SERIF_BOLD: &[u8] = include_bytes!("../../lib/doc_view/assets/DejaVuSerif-Bold.ttf");
static MONO_BOLD: &[u8] = include_bytes!("../../lib/doc_view/assets/DejaVuSansMono-Bold.ttf");
static UI: &[u8] = include_bytes!("../../styles/winui/assets/selawk.ttf");

/// the repository's fonts for the substitutes of a drawing's fonts
pub fn font(f: Family) -> Option<&'static [u8]> {
    Some(match f {
        Family::Sans => SANS,
        Family::SansBold => SANS_BOLD,
        Family::Serif => SERIF,
        Family::SerifBold => SERIF_BOLD,
        Family::Mono => MONO,
        Family::MonoBold => MONO_BOLD,
        Family::Ui => UI,
        _ => SANS,
    })
}

static FONTS: fn(Family) -> Option<&'static [u8]> = font;

/// the options of these tests and the viewer: the background, the repository's fonts
pub fn options(bg: u32, groups: bool) -> Options<'static> {
    let mut o = Options::new();
    o.background = bg | 0xFF00_0000;
    o.fonts = Some(&FONTS);
    o.groups = groups;
    o
}

/// a camera that shows a box in a target, 4 % margin
pub fn fit(b: [f64; 4], size: (u32, u32)) -> Camera {
    let (w, h) = ((b[2] - b[0]).max(1e-9), (b[3] - b[1]).max(1e-9));
    let scale = (w / size.0 as f64).max(h / size.1 as f64) * 1.08;
    Camera::new([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0], scale)
}

// ---------------------------------------------------------------------------------------------------- fixtures

/// a fixture as "CODE VALUE" lines into a DXF with the given line end
pub fn dxf_text(src: &str, eol: &str) -> String {
    let mut s = String::new();
    for line in src.lines() {
        let line = line.trim_start();
        if line.is_empty() {
            continue;
        }
        let (c, v) = match line.find(' ') {
            Some(i) => (&line[..i], &line[i + 1..]),
            None => (line, ""),
        };
        s.push_str(&format!("{c:>3}{eol}{v}{eol}"));
    }
    s
}

/// every kind this reader draws, the tables, a block with entities on other layers, a dimension's block, OCS (0, 0, -1),
/// paper space, an unsupported kind
pub const FIXTURE: &str = r"
0 SECTION
2 HEADER
9 $ACADVER
1 AC1015
9 $DWGCODEPAGE
3 ANSI_1252
9 $INSUNITS
70 4
9 $LTSCALE
40 2.0
9 $LWDISPLAY
290 1
9 $EXTMIN
10 0
20 0
30 0
9 $EXTMAX
10 100
20 60
30 0
0 ENDSEC
0 SECTION
2 TABLES
0 TABLE
2 LTYPE
0 LTYPE
2 CONTINUOUS
70 0
3 Solid line
72 65
73 0
40 0.0
0 LTYPE
2 DASHED
70 0
3 __ __ __
72 65
73 2
40 0.75
49 0.5
74 0
49 -0.25
74 0
0 LTYPE
2 DASHDOT
70 0
3 _ . _ .
72 65
73 4
40 1.0
49 0.5
49 -0.25
49 0.0
49 -0.25
0 LTYPE
2 GAS
70 0
3 Gas line ---- GAS ----
72 65
73 3
40 1.5
49 1.0
74 0
49 -0.25
74 2
75 0
46 0.1
50 0.0
44 -0.1
45 -0.05
9 GAS
49 -0.25
74 0
0 ENDTAB
0 TABLE
2 LAYER
0 LAYER
2 0
70 0
62 7
6 CONTINUOUS
0 LAYER
2 Walls
70 0
62 1
6 CONTINUOUS
370 50
0 LAYER
2 Hidden
70 0
62 -3
6 DASHED
0 LAYER
2 Frozen
70 1
62 5
6 CONTINUOUS
0 LAYER
2 Glass
70 0
62 4
6 CONTINUOUS
1001 AcCmTransparency
1071 33554559
0 ENDTAB
0 TABLE
2 STYLE
0 STYLE
2 Standard
70 0
40 0.0
41 1.0
50 0.0
71 0
42 2.5
3 txt
4
0 STYLE
2 Arial
70 0
40 0.0
41 1.0
50 0.0
71 0
3 arial.ttf
4
0 ENDTAB
0 ENDSEC
0 SECTION
2 BLOCKS
0 BLOCK
8 0
2 DOOR
70 0
10 1.0
20 1.0
30 0.0
3 DOOR
1
0 LINE
8 0
62 0
10 1.0
20 1.0
30 0
11 1.0
21 9.0
31 0
0 ARC
8 Walls
10 1.0
20 1.0
30 0
40 8.0
50 0.0
51 90.0
0 LINE
8 Frozen
10 1
20 1
11 9
21 1
0 ENDBLK
8 0
0 BLOCK
8 0
2 *D1
70 1
10 0
20 0
30 0
3 *D1
1
0 LINE
8 0
10 10
20 50
11 40
21 50
0 SOLID
8 0
10 10
20 50
11 12
21 50.5
12 12
22 49.5
13 12
23 49.5
0 MTEXT
8 0
10 25
20 51.5
40 1.5
71 8
1 30
0 ENDBLK
8 0
0 ENDSEC
0 SECTION
2 ENTITIES
0 LINE
8 0
10 0
20 0
11 30
21 0
0 LINE
8 Walls
6 DASHED
10 0
20 2
11 29.6
21 2
0 CIRCLE
8 Walls
10 50
20 10
40 5
0 ARC
8 0
62 3
10 50
20 10
40 7
50 30
51 300
0 ELLIPSE
8 0
10 70
20 10
11 8
21 0
31 0
40 0.5
41 0
42 6.283185307179586
0 LWPOLYLINE
8 0
62 6
90 4
70 1
10 80
20 0
42 1
10 90
20 0
10 90
20 10
42 -0.4142135623730951
10 80
20 10
0 LWPOLYLINE
8 Hidden
90 3
70 0
10 0
20 20
10 10
20 25
10 20
20 20
0 LWPOLYLINE
8 0
6 DASHED
90 3
70 128
10 0
20 30
42 0.5
10 10
20 30
10 20
20 35
0 LWPOLYLINE
8 0
62 2
90 3
70 0
43 0.8
10 30
20 30
10 40
20 30
42 0.3
10 45
20 35
0 LWPOLYLINE
8 0
62 1
90 2
70 0
10 50
20 30
40 2.0
41 0.0
10 58
20 30
0 POLYLINE
8 Walls
66 1
10 0
20 0
30 0
70 0
0 VERTEX
8 Walls
10 60
20 40
42 0.5
0 VERTEX
8 Walls
10 70
20 40
0 VERTEX
8 Walls
10 70
20 45
0 SEQEND
8 Walls
0 POINT
8 0
10 75
20 45
0 SPLINE
8 0
70 8
71 3
72 8
73 4
74 0
40 0
40 0
40 0
40 0
40 1
40 1
40 1
40 1
10 80
20 40
30 0
10 83
20 48
30 0
10 87
20 32
30 0
10 90
20 40
30 0
0 SPLINE
8 0
70 8
71 3
72 0
73 0
74 3
11 80
21 50
31 0
11 85
21 55
31 0
11 90
21 50
31 0
0 TEXT
8 0
10 2
20 45
40 2.0
1 Wall 45%%d %%uunderlined%%u
0 TEXT
8 0
7 Arial
10 2
20 50
11 22
21 50
40 1.5
72 3
1 Aligned
0 MTEXT
8 0
10 30
20 58
40 1.2
41 25
71 1
1 Plain MTEXT\Pnext line
0 MTEXT
8 0
10 30
20 50
40 1.0
41 0
71 1
1 {\H1.5x;Big} {\C1;red} \S1/2; {\Lunder\l} {\fTimes New Roman|b1|i0;Serif}
0 INSERT
8 Walls
2 DOOR
10 0
20 60
41 1
42 1
50 0
70 2
71 3
44 12
45 12
0 INSERT
8 0
66 1
2 DOOR
10 40
20 60
41 -1
42 1
50 30
0 ATTRIB
8 0
10 40
20 62
40 1
1 D-01
2 TAG
70 0
0 SEQEND
8 0
0 HATCH
8 0
62 3
10 0
20 0
30 0
210 0
220 0
230 1
2 SOLID
70 1
71 0
91 1
92 3
72 1
73 1
93 4
10 60
20 0
42 0
10 70
20 0
42 0.5
10 70
20 8
42 0
10 60
20 8
42 0
97 0
75 0
76 1
98 0
0 HATCH
8 0
62 5
10 0
20 0
30 0
2 ANSI31
70 0
71 0
91 1
92 1
93 4
72 1
10 72
20 0
11 79
21 0
72 2
10 79
20 4
40 4
50 270
51 90
73 1
72 1
10 79
20 8
11 72
21 8
72 1
10 72
20 8
11 72
21 0
97 0
75 0
76 1
52 0
41 1
77 0
78 1
53 45
43 0
44 0
45 -0.88388
46 0.88388
79 0
98 0
0 HATCH
8 0
62 1
10 0
20 0
30 0
2 GRID5
70 0
71 0
91 1
92 2
72 0
73 1
93 4
10 0
20 -10
10 10
20 -10
10 10
20 -5
10 0
20 -5
97 0
75 0
76 1
52 0
41 1
77 0
78 5
53 0
43 0
44 0
45 0
46 1
79 0
53 90
43 0
44 0
45 -1
46 0
79 0
53 45
43 0
44 0
45 -1
46 1
79 2
49 0.5
49 -0.5
53 135
43 0
44 0
45 -1
46 -1
79 0
53 30
43 0
44 0
45 -0.5
46 0.866
79 0
98 0
0 SOLID
8 0
62 4
10 12
20 -10
11 18
21 -10
12 12
22 -5
13 18
23 -5
0 3DFACE
8 0
62 5
10 20
20 -10
30 0
11 26
21 -10
31 0
12 26
22 -5
32 0
13 20
23 -5
33 0
0 DIMENSION
8 0
2 *D1
10 40
20 50
11 25
21 51.5
70 0
0 LEADER
8 0
71 1
72 0
76 2
10 30
20 -5
10 40
20 -2
0 WIPEOUT
8 0
10 45
20 -10
11 5
21 0
12 0
22 5
13 1
23 1
71 1
91 2
14 -0.5
24 -0.5
14 0.5
24 0.5
0 CIRCLE
8 0
210 0
220 0
230 -1
10 55
20 -5
40 3
0 ARC
8 0
210 0
220 0
230 -1
10 55
20 -5
40 4
50 0
51 90
0 IMAGE
8 0
10 0
20 0
0 LINE
8 0
67 1
10 0
20 0
11 1
21 1
0 VIEWPORT
8 0
67 1
10 0
20 0
0 LINE
8 Glass
10 0
20 -12
11 30
21 -12
0 ENDSEC
0 EOF";

/// an R12 file without a header: ENTITIES only, POLYLINE with VERTEXes
pub const R12: &str = r"
0 SECTION
2 ENTITIES
0 POLYLINE
8 0
66 1
70 1
0 VERTEX
8 0
10 0
20 0
0 VERTEX
8 0
10 10
20 0
42 1
0 VERTEX
8 0
10 10
20 10
0 SEQEND
0 TEXT
8 0
10 0
20 -3
40 1
1 R12 %%c10
0 ENDSEC
0 EOF";

/// a section's name without "0 SECTION" (as GDAL's text.dxf), no EOF
pub const NO_SECTION: &str = r"
0 SECTION
2 TABLES
0 ENDSEC
2 ENTITIES
0 LINE
8 0
10 0
20 0
11 1
21 1
0 ENDSEC";

// ---------------------------------------------------------------------------------------------------- an independent count

/// the entity markers of a file's ENTITIES and BLOCKS sections, counted line by line without the reader: every "0" pair
/// there but ENDSEC (lines end in LF, CR LF or a lone CR)
pub fn markers(b: &[u8]) -> Vec<((dxf::Where, String), u64)> {
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\n' || b[i] == b'\r' {
            lines.push(&b[start..i]);
            if b[i] == b'\r' && i + 1 < b.len() && b[i + 1] == b'\n' {
                i += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < b.len() {
        lines.push(&b[start..]);
    }
    let trim = |s: &[u8]| -> String { String::from_utf8_lossy(s).trim().to_ascii_uppercase() };
    let mut out: Vec<((dxf::Where, String), u64)> = Vec::new();
    let mut sec: Option<dxf::Where> = None;
    // (the pair before: a section's name follows 0 SECTION, or 0 ENDSEC where a writer left SECTION out)
    let mut prev = (String::new(), String::new());
    let mut k = 0;
    while k + 1 < lines.len() {
        let (c, v) = (trim(lines[k]), trim(lines[k + 1]));
        let before = core::mem::replace(&mut prev, (c.clone(), v.clone()));
        if c == "0" {
            if v == "ENDSEC" {
                sec = None;
            } else if v == "EOF" {
                break;
            } else if let Some(w) = sec {
                match out.iter_mut().find(|x| x.0 .0 == w && x.0 .1 == v) {
                    Some(x) => x.1 += 1,
                    None => out.push(((w, v), 1)),
                }
            }
        } else if c == "2" && before.0 == "0" && (before.1 == "SECTION" || before.1 == "ENDSEC") {
            sec = match v.as_str() {
                "ENTITIES" => Some(dxf::Where::Entities),
                "BLOCKS" => Some(dxf::Where::Blocks),
                _ => None,
            };
        }
        k += 2;
    }
    out
}

// ---------------------------------------------------------------------------------------------------- --test dxf

fn rng_next(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// four small drawings of ezdxf (MIT; apps/render2dtest/dxf/README.md says where each comes from)
const EZ_CW_HATCH: &[u8] = include_bytes!("dxf/clockwise_arcs_hatch.dxf");
const EZ_CARET: &[u8] = include_bytes!("dxf/caret_encoding.dxf");
const EZ_MTEXT: &[u8] = include_bytes!("dxf/mtext_complex_inline_codes.dxf");
const EZ_BIN_R12: &[u8] = include_bytes!("dxf/bin_dxf_r12.dxf");

/// render2dtest --test dxf
pub fn dxf(t: &mut T) {
    reader(t);
    mapping(t);
    crafted(t);
    ezdxf_files(t);
    gpu_cpu(t);
}

/// the text of a parsed MTEXT, its paragraphs one line each, a tab as '\t'
fn mtext_plain(m: &dxf::mtext::Mtext) -> String {
    let mut s = String::new();
    for (i, p) in m.paragraphs.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        for it in &p.items {
            match it {
                dxf::mtext::Item::Text(x, _) => s.push_str(x),
                dxf::mtext::Item::Tab(_) => s.push('\t'),
                _ => {}
            }
        }
    }
    s
}

/// what the drawings of ezdxf show that the test's own drawings do not
fn ezdxf_files(t: &mut T) {
    use render2d::math::sincos;
    t.ok(dxf::is_binary(EZ_BIN_R12) && matches!(dxf::load(EZ_BIN_R12, &dxf::Limits::DEFAULT), Err(dxf::Error::Binary)), "ezdxf bin_dxf_r12.dxf: Error::Binary");
    // clockwise arcs in a hatch's boundary: DXF stores their angles mirrored (the arc runs from -start clockwise to
    // -end), and only so does each path's chain of edges close; read as stored, it does not
    match dxf::load(EZ_CW_HATCH, &dxf::Limits::DEFAULT) {
        Ok(doc) => {
            let hatch = doc.entities.iter().find_map(|e| if let dxf::Kind::Hatch(h) = &e.kind { Some(h) } else { None });
            let gaps = |mirrored: bool| -> (usize, usize, f64) {
                let (mut edges, mut cw, mut worst) = (0usize, 0usize, 0.0f64);
                for p in hatch.map_or(&[][..], |h| &h.paths[..]) {
                    let ends: Vec<([f64; 2], [f64; 2])> = p.edges.iter().filter_map(|e| match e {
                        dxf::Edge::Line { a, b } => Some((*a, *b)),
                        dxf::Edge::Arc { center, radius, start, end, ccw } => {
                            let (a0, a1) = if *ccw || !mirrored { (*start, *end) } else { (-*start, -*end) };
                            cw += !*ccw as usize;
                            let (s0, c0) = sincos(a0.to_radians());
                            let (s1, c1) = sincos(a1.to_radians());
                            let p0 = [center[0] + radius * c0, center[1] + radius * s0];
                            let p1 = [center[0] + radius * c1, center[1] + radius * s1];
                            Some((p0, p1))
                        }
                        _ => None,
                    }).collect();
                    edges += ends.len();
                    for i in 0..ends.len() {
                        let (b, a) = (ends[i].1, ends[(i + 1) % ends.len()].0);
                        worst = worst.max((b[0] - a[0]).abs().max((b[1] - a[1]).abs()));
                    }
                }
                (edges, cw, worst)
            };
            let (n, cw, good) = gaps(true);
            let (_, _, naive) = gaps(false);
            t.ok(hatch.is_some_and(|h| h.paths.len() == 2) && n == 37 && cw == 4 && good < 1e-6 && naive > 1.0,
                 &format!("ezdxf clockwise_arcs_hatch.dxf: 2 paths, {n} edges ({cw} clockwise arcs) close up to {good:.1e} with the mirrored angles, {naive:.2} apart as stored"));
            let mut s = Scene::new_cpu();
            let c = render2d::dxf::load(&mut s, EZ_CW_HATCH, &options(BG, false)).ok().map(|l| l.kinds.get("HATCH").copied().unwrap_or_default());
            t.ok(c.is_some_and(|c| c.read == 1 && c.drawn == 1 && c.failed == 0), &format!("ezdxf clockwise_arcs_hatch.dxf into a scene: the hatch drawn ({:?})", c.map(|c| (c.read, c.drawn, c.failed))));
        }
        Err(e) => {
            t.ok(false, &format!("ezdxf clockwise_arcs_hatch.dxf: {e}"));
        }
    }
    // caret codes in MTEXT: ^J a new paragraph, ^I a tab, "^ " a caret
    match dxf::load(EZ_CARET, &dxf::Limits::DEFAULT) {
        Ok(doc) => {
            let texts: Vec<String> = doc.entities.iter().filter_map(|e| if let dxf::Kind::MText(m) = &e.kind { Some(mtext_plain(&dxf::mtext::parse(&m.text, m.height))) } else { None }).collect();
            let find = |head: &str| texts.iter().find(|x| x.starts_with(head)).cloned().unwrap_or_default();
            let (j, i, sp) = (find("J: "), find("I: "), find(" : "));
            t.ok(texts.len() == 61 && j == "J: abc\ndef" && i == "I: abc\tdef" && sp.ends_with("abc^def"),
                 &format!("ezdxf caret_encoding.dxf: {} MTEXT; ^J {j:?}, ^I {i:?}, \"^ \" {sp:?}", texts.len()));
            let mut s = Scene::new_cpu();
            let c = render2d::dxf::load(&mut s, EZ_CARET, &options(BG, false)).ok().map(|l| l.kinds.get("MTEXT").copied().unwrap_or_default());
            t.ok(c.is_some_and(|c| c.read == 61 && c.drawn == 61 && c.failed == 0), &format!("ezdxf caret_encoding.dxf into a scene: every MTEXT drawn ({:?})", c.map(|c| (c.read, c.drawn, c.failed))));
        }
        Err(e) => {
            t.ok(false, &format!("ezdxf caret_encoding.dxf: {e}"));
        }
    }
    // MTEXT with many inline codes and four mirrorings: all of it drawn
    let mut s = Scene::new_cpu();
    match render2d::dxf::load(&mut s, EZ_MTEXT, &options(BG, false)) {
        Ok(l) => {
            let (m, ln) = (l.kinds.get("MTEXT").copied().unwrap_or_default(), l.kinds.get("LINE").copied().unwrap_or_default());
            t.ok(m.read == 8 && m.drawn == 8 && m.failed == 0 && ln.read == 4 && ln.drawn == 4,
                 &format!("ezdxf mtext_complex_inline_codes.dxf: MTEXT {} of {} drawn, LINE {} of {}", m.drawn, m.read, ln.drawn, ln.read));
        }
        Err(e) => {
            t.ok(false, &format!("ezdxf mtext_complex_inline_codes.dxf: {e}"));
        }
    }
}

fn reader(t: &mut T) {
    let text = dxf_text(FIXTURE, "\r\n");
    let doc = match dxf::load(text.as_bytes(), &dxf::Limits::DEFAULT) {
        Ok(d) => d,
        Err(e) => {
            t.ok(false, &format!("the fixture reads: {e}"));
            return;
        }
    };
    t.ok(doc.header.version == "AC1015" && doc.header.ltscale == 2.0 && doc.header.lwdisplay == Some(true) && doc.header.insunits == 4 && doc.header.extmax == Some([100.0, 60.0, 0.0]),
         &format!("the header: {} $LTSCALE {} $LWDISPLAY {:?} $INSUNITS {} $EXTMAX {:?}", doc.header.version, doc.header.ltscale, doc.header.lwdisplay, doc.header.insunits, doc.header.extmax));
    let l = |n: &str| doc.names.find(n).and_then(|n| doc.tables.layer(n).copied());
    let (walls, hidden, frozen, glass) = (l("WALLS"), l("hidden"), l("Frozen"), l("Glass"));
    t.ok(walls.is_some_and(|w| w.color == dxf::Color::Aci(1) && w.lineweight == dxf::Lineweight::Mm100(50)) && hidden.is_some_and(|h| !h.on && h.color == dxf::Color::Aci(3))
         && frozen.is_some_and(|f| f.frozen) && glass.is_some_and(|g| g.transparency == Some(127)),
         &format!("layers (names without case): {} of 5 (Walls red 0.50 mm, Hidden off, Frozen, Glass 50 %)", doc.tables.layers.len()));
    let gas = doc.names.find("GAS").and_then(|n| doc.tables.linetype(n));
    t.ok(gas.is_some_and(|g| g.dashes.len() == 3 && g.dashes[1].complex == 2 && g.dashes[1].text.as_deref() == Some("GAS")),
         "a complex linetype: its text element kept (74 = 2, 9 GAS)");
    let door = doc.names.find("DOOR").and_then(|n| doc.block(n));
    t.ok(door.is_some_and(|b| b.base == [1.0, 1.0, 0.0] && b.entities.len() == 3), "the block DOOR: base (1, 1), 3 entities");
    // the kinds against the independent count
    let ind = markers(text.as_bytes());
    let mut same = true;
    for ((w, k), n) in &ind {
        if doc.stats.count(*w, k) != *n {
            same = false;
            out(&format!("    {w:?} {k}: reader {} independent {n}", doc.stats.count(*w, k)));
        }
    }
    t.ok(same && ind.len() > 20, &format!("the reader's kinds against an independent count of the markers: {} kinds alike", ind.len()));
    // the same file with other line ends, and handed out a byte (or seven) at a time
    for (eol, what) in [("\n", "LF"), ("\r", "a lone CR")] {
        let other = dxf_text(FIXTURE, eol);
        let d2 = dxf::load(other.as_bytes(), &dxf::Limits::DEFAULT);
        t.ok(d2.as_ref().is_ok_and(|d| d.entities.len() == doc.entities.len() && d.blocks.len() == doc.blocks.len() && d.stats.kinds == doc.stats.kinds),
             &format!("line ends {what}: the same entities"));
    }
    for step in [1usize, 7, 4093] {
        let b = text.as_bytes();
        let mut at = 0usize;
        let mut src = |buf: &mut [u8]| {
            let n = step.min(buf.len()).min(b.len() - at);
            buf[..n].copy_from_slice(&b[at..at + n]);
            at += n;
            n
        };
        struct Count(usize, usize);
        impl dxf::Sink for Count {
            fn block(&mut self, _b: dxf::Block, _: &dxf::Names) {
                self.1 += 1;
            }
            fn entity(&mut self, _e: dxf::Entity, _: &dxf::Names) {
                self.0 += 1;
            }
        }
        let mut c = Count(0, 0);
        let r = dxf::read(&mut src, &dxf::Limits::DEFAULT, &mut c);
        t.ok(r.as_ref().is_ok_and(|(s, _)| s.kinds == doc.stats.kinds) && c.0 == doc.entities.len() && c.1 == doc.blocks.len(),
             &format!("streamed in pieces of {step} bytes: {} entities, {} blocks, the same kinds", c.0, c.1));
    }
    // without a header, R12
    let r12 = dxf::load(dxf_text(R12, "\r\n").as_bytes(), &dxf::Limits::DEFAULT);
    let ok12 = r12.as_ref().is_ok_and(|d| {
        !d.header.present
            && matches!(&d.entities[0].kind, dxf::Kind::Polyline(p) if p.vertices.len() == 3 && p.closed && p.vertices[1].bulge == 1.0)
            && matches!(&d.entities[1].kind, dxf::Kind::Text(x) if x.text == "R12 %%c10")
    });
    t.ok(ok12, "R12 without a header: a closed POLYLINE of three VERTEXes with a bulge, a TEXT");
    let ns = dxf::load(dxf_text(NO_SECTION, "\n").as_bytes(), &dxf::Limits::DEFAULT);
    t.ok(ns.as_ref().is_ok_and(|d| d.entities.len() == 1 && !d.stats.notes.is_empty()), "a section's name without 0 SECTION, no EOF: read, with a note");
    // code pages and escapes
    let cp = |ver: &str, page: &str, bytes: &[u8]| -> String {
        let mut v = dxf_text(&format!("0 SECTION\n2 HEADER\n9 $ACADVER\n1 {ver}\n9 $DWGCODEPAGE\n3 {page}\n0 ENDSEC\n0 SECTION\n2 ENTITIES\n0 TEXT\n8 0\n40 1"), "\n").into_bytes();
        v.extend_from_slice(b"  1\n");
        v.extend_from_slice(bytes);
        v.extend_from_slice(b"\n  0\nENDSEC\n  0\nEOF\n");
        match dxf::load(&v, &dxf::Limits::DEFAULT).ok().and_then(|d| d.entities.into_iter().next()) {
            Some(dxf::Entity { kind: dxf::Kind::Text(x), .. }) => x.text,
            _ => String::from("?"),
        }
    };
    let a = cp("AC1015", "ANSI_1251", &[0xC0, 0xE1]);
    let b = cp("AC1015", "ANSI_1252", &[0x45, 0xE9, 0x80]);
    let c = cp("AC1021", "ANSI_1252", "Größe \\U+2300".as_bytes());
    let d = cp("AC1015", "ANSI_932", &[0x41, 0x82, 0xA0]);
    t.ok(a == "Аб" && b == "Eé€" && c == "Größe ⌀" && d == "A\u{FFFD}\u{FFFD}", &format!("code pages: ANSI_1251 {a:?}, ANSI_1252 {b:?}, AC1021 UTF-8 with \\U+ {c:?}, ANSI_932 (no table) {d:?}"));
    // TEXT's %% codes, MTEXT's format codes
    let sp = dxf::specials("Wall 45%%d %%uunder%%u %%c%%p%%%%%065");
    t.ok(sp.len() == 3 && sp[0].text == "Wall 45° " && sp[1].underline && sp[1].text == "under" && sp[2].text == " ∅±%A", &format!("TEXT's %% codes: {sp:?}"));
    let mt = dxf::mtext::parse("{\\H1.5x;Big} {\\C1;red} \\S1/2; {\\Lunder\\l}\\P\\pxqc;{\\fTimes New Roman|b1|i0;Serif}\\~x^Iy", 2.0);
    let items = &mt.paragraphs[0].items;
    let big = matches!(&items[0], dxf::mtext::Item::Text(s, f) if s == "Big" && f.height == 3.0);
    let red = items.iter().any(|i| matches!(i, dxf::mtext::Item::Text(s, f) if s == "red" && f.color == Some(dxf::Color::Aci(1))));
    let stack = items.iter().any(|i| matches!(i, dxf::mtext::Item::Stack { top, bottom, kind: dxf::mtext::StackKind::Bar, .. } if top == "1" && bottom == "2"));
    let under = items.iter().any(|i| matches!(i, dxf::mtext::Item::Text(s, f) if s == "under" && f.underline));
    let p2 = mt.paragraphs.get(1);
    let serif = p2.is_some_and(|p| p.align == dxf::mtext::Align::Center && matches!(&p.items[0], dxf::mtext::Item::Text(s, f) if s == "Serif" && f.font.as_deref() == Some("Times New Roman") && f.bold));
    let tab = p2.is_some_and(|p| p.items.iter().any(|i| matches!(i, dxf::mtext::Item::Tab(_))));
    t.ok(big && red && stack && under && serif && tab && mt.plain(2.0).is_none() && dxf::mtext::parse("a\\Pb", 2.0).plain(2.0).as_deref() == Some("a\nb"),
         &format!("MTEXT's codes: \\H, \\C, \\S, \\L, \\P, \\pxqc, \\f, ^I ({} paragraphs)", mt.paragraphs.len()));
    // AutoCAD's colour index
    let aci = [(1u8, 0xFF0000u32), (8, 0x808080), (9, 0xC0C0C0), (10, 0xFF0000), (11, 0xFF7F7F), (12, 0xCC0000), (21, 0xFF9F7F), (30, 0xFF7F00), (130, 0x00FFFF), (254, 0xBEBEBE)];
    let bad: Vec<String> = aci.iter().filter(|(i, v)| dxf::aci_rgb(*i) != *v).map(|(i, v)| format!("{i}: {:06X} not {v:06X}", dxf::aci_rgb(*i))).collect();
    t.ok(bad.is_empty(), &format!("AutoCAD's colour index: {} of {} spot checks{}", aci.len() - bad.len(), aci.len(), if bad.is_empty() { String::new() } else { format!(" ({})", bad.join(", ")) }));
    // font substitutes
    let subs = [("txt", Family::Sans, true), ("simplex.shx", Family::Sans, true), ("monotxt.shx", Family::Mono, true), ("arial.ttf", Family::Sans, false), ("ARIALN.TTF", Family::Sans, false),
                ("timesbd.ttf", Family::SerifBold, false), ("cour.ttf", Family::Mono, false), ("segoeui.ttf", Family::Ui, false), ("ariblk.ttf", Family::SansBold, false)];
    let bad: Vec<String> = subs.iter().filter(|(n, f, shx)| { let s = dxf::font_substitute(n, None); s.family != *f || s.shx != *shx }).map(|(n, _, _)| String::from(*n)).collect();
    t.ok(bad.is_empty(), &format!("font substitutes: {} of {}{}", subs.len() - bad.len(), subs.len(), if bad.is_empty() { String::new() } else { format!(" (wrong: {})", bad.join(", ")) }));
    // linetype patterns: signed DXF entries into render2d's
    let pat = |s: &[f64], sc: f64, max: usize| render2d::dxf::linetype_pattern(s, sc, max);
    let p1 = pat(&[0.5, -0.25, 0.0, -0.25], 2.0, 8);
    let p2 = pat(&[-0.25, 0.5], 1.0, 8);
    let p3 = pat(&[12.0, -3.0, 12.0, -3.0, 0.0, -3.0, 0.0, -3.0, 0.0, -3.0], 1.0, 8);
    let p4 = pat(&[1.0], 1.0, 8);
    t.ok(p1 == Some((vec![1.0, 0.5, 0.0, 0.5], 0.0)) && p2 == Some((vec![0.5, 0.25], 0.5)) && p3.as_ref().is_some_and(|(v, _)| v.len() == 8 && (v.iter().sum::<f64>() - 39.0).abs() < 1e-9) && p4.is_none(),
         &format!("linetype patterns: {p1:?}, a leading gap {p2:?}, ten entries into eight {p3:?}, a dash alone (solid) {p4:?}"));
}

fn mapping(t: &mut T) {
    let text = dxf_text(FIXTURE, "\r\n");
    let mut s = Scene::new_cpu();
    let o = options(BG, true);
    let l = match render2d::dxf::load(&mut s, text.as_bytes(), &o) {
        Ok(l) => l,
        Err(e) => {
            t.ok(false, &format!("the fixture into a scene: {e}"));
            return;
        }
    };
    let k = |n: &str| l.kinds.get(n).copied().unwrap_or_default();
    for (kind, read, drawn) in [("LINE", 7, 5), ("CIRCLE", 2, 2), ("ARC", 3, 3), ("ELLIPSE", 1, 1), ("LWPOLYLINE", 5, 5), ("POLYLINE", 1, 1), ("POINT", 1, 1), ("SPLINE", 2, 2), ("TEXT", 2, 2),
                                ("MTEXT", 3, 3), ("INSERT", 2, 2), ("HATCH", 3, 3), ("SOLID", 2, 2), ("3DFACE", 1, 1), ("DIMENSION", 1, 1), ("LEADER", 1, 1), ("WIPEOUT", 1, 1)] {
        let c = k(kind);
        t.ok(c.read == read && c.drawn == drawn && c.failed == 0, &format!("{kind}: read {} (want {read}), drawn {} (want {drawn}), failed {}, approximated {}", c.read, c.drawn, c.failed, c.approximated));
    }
    let (line, img, vp) = (k("LINE"), k("IMAGE"), k("VIEWPORT"));
    t.ok(line.paper == 1 && line.hidden == 1 && img.unsupported == 1 && vp.paper == 1,
         &format!("paper space skipped (LINE {}, VIEWPORT {}), a block's entity on a frozen layer hidden ({}), IMAGE unsupported ({})", line.paper, vp.paper, line.hidden, img.unsupported));
    t.ok(k("SPLINE").approximated == 1 && k("HATCH").approximated == 1, "approximated: the spline of fit points, the hatch of five line families");
    let e = l.extents.unwrap_or([0.0; 4]);
    t.ok(e[0] < -58.9 && e[1] < -11.9 && (e[2] - 90.0).abs() < 1e-9 && e[3] > 60.0, &format!("the extents of what was drawn: {e:?}"));
    let hidden = l.layers.iter().find(|x| x.name == "Hidden").map(|x| x.visible);
    t.ok(hidden == Some(false) && l.layers.iter().filter(|x| x.name == "0").count() == 1 && l.layers.iter().map(|x| x.runs.len()).sum::<usize>() > l.layers.len(),
         &format!("{} DXF layers in {} runs (the file's order), Hidden not shown", l.layers.len(), l.layers.iter().map(|x| x.runs.len()).sum::<usize>()));
    // the groups: a polyline of bulges is several render2d entities, picked as one
    let ents = l.entities;
    let grouped = (1..200u32).filter(|&g| l.members(g).len() > 1).count();
    t.ok(grouped > 0 && ents > 40, &format!("{ents} render2d entities; {grouped} DXF entities became several (one group each)"));
    let mut l2 = l.clone();
    let shown = l2.set_layer_visible(&mut s, "walls", false);
    t.ok(shown.is_ok_and(|b| b) && l2.layers.iter().filter(|x| x.name == "Walls").all(|x| !x.visible), "set_layer_visible(\"walls\", false): its runs hidden");
    out(&format!("    notes: {}", l.notes.join(" | ")));
}

fn crafted(t: &mut T) {
    // a binary DXF, not a DXF, a code that is no number in the middle, cut off in an entity, a line too long, limits
    let bin = b"AutoCAD Binary DXF\r\n\x1a\x00\x00\x00rest";
    t.ok(matches!(dxf::load(bin, &dxf::Limits::DEFAULT), Err(dxf::Error::Binary)), "a binary DXF: Error::Binary");
    let mut s = Scene::new_cpu();
    t.ok(matches!(render2d::dxf::load(&mut s, bin, &options(BG, false)), Err(dxf::Error::Binary)), "render2d::dxf::load of a binary DXF: Error::Binary");
    t.ok(matches!(dxf::load(b"***NEWFILE***:my.shp\r\nSECTION", &dxf::Limits::DEFAULT), Err(dxf::Error::Invalid(_))), "not a DXF: Error::Invalid");
    let text = dxf_text(FIXTURE, "\n");
    let mut broken = text.clone();
    if let Some(i) = broken.find("\n 10\n50\n") {
        broken.replace_range(i + 1..i + 4, "1x0");
    }
    let part = dxf::load(broken.as_bytes(), &dxf::Limits::DEFAULT);
    t.ok(part.as_ref().is_ok_and(|d| d.stats.notes.iter().any(|n| n.contains("the rest of the file is not read")) && d.entities.len() < 34),
         &format!("a group code that is no number in the middle: what came before it, and a note ({} entities)", part.as_ref().map_or(0, |d| d.entities.len())));
    let cut = &text.as_bytes()[..text.len() * 2 / 3];
    let c = dxf::load(cut, &dxf::Limits::DEFAULT);
    t.ok(c.is_ok(), &format!("cut off two thirds in: what was there ({} entities)", c.as_ref().map_or(0, |d| d.entities.len())));
    let mut long = String::from("  0\nSECTION\n  2\nENTITIES\n  0\nTEXT\n  1\n");
    long.push_str(&"x".repeat(2 << 20));
    let mut lim = dxf::Limits::DEFAULT;
    t.ok(matches!(dxf::load(long.as_bytes(), &lim), Err(dxf::Error::Limit(_))), "a line of 2 MiB: Error::Limit");
    // a CR as the last byte of the window grown to its most (line_bytes + 1 MiB): Error::Limit, not a wait for its LF
    let mut edge = String::from("  0\nSECTION\n  2\nENTITIES\n  0\nTEXT\n  1\n");
    edge.push_str(&"x".repeat((2 << 20) - 1));
    edge.push_str("\r\n  0\nENDSEC\n  0\nEOF\n");
    let t0 = plat::time_us();
    let r = dxf::load(edge.as_bytes(), &lim);
    t.ok(matches!(r, Err(dxf::Error::Limit(_))), &format!("a line of 2 MiB whose CR ends the full window: Error::Limit ({} ms)", (plat::time_us() - t0) / 1000));
    lim.entity_points = 5;
    t.ok(matches!(dxf::load(text.as_bytes(), &lim), Err(dxf::Error::Limit(_))), "an entity of more points than Limits::entity_points: Error::Limit");
    let mut lim = dxf::Limits::DEFAULT;
    lim.kept_entities = 10;
    t.ok(matches!(dxf::load(text.as_bytes(), &lim), Err(dxf::Error::Limit(_))), "more entities kept than Limits::kept_entities: Error::Limit");
    // a vertex count of 2^31, a hatch claiming a million paths: lists grow as their items come, nothing allocated by them
    let lies = dxf_text("0 SECTION\n2 ENTITIES\n0 LWPOLYLINE\n90 2147483647\n10 0\n20 0\n10 1\n20 1\n0 HATCH\n91 1000000\n92 2\n93 2000000000\n10 0\n20 0\n0 SPLINE\n71 999\n72 2000000000\n40 0\n10 0\n20 0\n0 ENDSEC\n", "\n");
    let mut s = Scene::new_cpu();
    let r = render2d::dxf::load(&mut s, lies.as_bytes(), &options(BG, false));
    t.ok(r.is_ok(), &format!("counts that lie (2^31 vertices, a million paths, 2e9 knots, degree 999): read without allocating by them ({:?})", r.as_ref().map(|l| l.entities).ok()));
    // a hatch of 1000 line families: at most 64 of them, in 16 fills of its boundary (not 250 copies of it)
    let mut many = String::from("0 SECTION\n2 ENTITIES\n0 HATCH\n8 0\n2 MANY\n70 0\n71 0\n91 1\n92 2\n72 0\n73 1\n93 4\n10 0\n20 0\n10 10\n20 0\n10 10\n20 10\n10 0\n20 10\n97 0\n75 0\n76 1\n52 0\n41 1\n77 0\n78 1000\n");
    for i in 0..1000 {
        many.push_str(&format!("53 0\n43 0\n44 0\n45 0\n46 {}\n79 0\n", 1.0 + i as f64 * 0.01));
    }
    many.push_str("98 0\n0 ENDSEC\n0 EOF\n");
    let mut s = Scene::new_cpu();
    let r = render2d::dxf::load(&mut s, dxf_text(&many, "\n").as_bytes(), &options(BG, false));
    let h = r.as_ref().map(|l| (l.entities, l.kinds.get("HATCH").copied().unwrap_or_default())).ok();
    t.ok(h.is_some_and(|(n, c)| n == 16 && c.drawn == 1 && c.approximated == 1),
         &format!("a hatch of 1000 line families: 64 of them in 16 fills ({:?} render2d entities)", h.map(|x| x.0)));
    // a spline of degree 5 and 20 000 control points: evaluated in one pass over its knots (from the first knot for each
    // of its 65 536 points it took about 6.5e8 steps)
    let n = 20_000usize;
    let mut sp = format!("0 SECTION\n2 ENTITIES\n0 SPLINE\n8 0\n70 8\n71 5\n72 {}\n73 {n}\n", n + 6);
    for k in 0..n + 6 {
        sp.push_str(&format!("40 {}\n", k.saturating_sub(5).min(n - 5)));
    }
    for k in 0..n {
        sp.push_str(&format!("10 {k}\n20 {}\n30 0\n", (k % 7) as f64));
    }
    sp.push_str("0 ENDSEC\n0 EOF\n");
    let mut s = Scene::new_cpu();
    let t0 = plat::time_us();
    let r = render2d::dxf::load(&mut s, dxf_text(&sp, "\n").as_bytes(), &options(BG, false));
    let ms = (plat::time_us() - t0) / 1000;
    let c = r.as_ref().map(|l| l.kinds.get("SPLINE").copied().unwrap_or_default()).ok();
    t.ok(c.is_some_and(|c| c.drawn == 1 && c.approximated == 1), &format!("a spline of degree 5 and {n} control points: evaluated into a polyline ({ms} ms)"));
    // random damage: bytes changed, cut, repeated; never a panic or a hang, and in bounded time
    let base = text.as_bytes();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let (mut oks, mut errs) = (0u32, 0u32);
    let t0 = plat::time_us();
    for round in 0..600u32 {
        let mut v = base.to_vec();
        let n = 1 + (rng_next(&mut seed) % 24) as usize;
        for _ in 0..n {
            let at = (rng_next(&mut seed) % v.len() as u64) as usize;
            match rng_next(&mut seed) % 5 {
                0 => v[at] = (rng_next(&mut seed) & 0xFF) as u8,
                1 => v[at] = b"0123456789-.eE\n\r "[(rng_next(&mut seed) % 17) as usize],
                2 => {
                    let l = ((rng_next(&mut seed) % 64) as usize).min(v.len() - at);
                    v.drain(at..at + l);
                }
                3 => {
                    let l = ((rng_next(&mut seed) % 128) as usize).min(v.len() - at);
                    let piece: Vec<u8> = v[at..at + l].to_vec();
                    v.splice(at..at, piece);
                }
                _ => {
                    // a number grows huge
                    let s = b"1e308";
                    v.splice(at..at, s.iter().copied());
                }
            }
            if v.is_empty() {
                v.push(b'0');
            }
        }
        // every third one through the whole mapping as well
        let r = if round % 3 == 0 {
            let mut s = Scene::new_cpu();
            render2d::dxf::load(&mut s, &v, &options(BG, round % 2 == 0)).map(|_| ())
        } else {
            dxf::load(&v, &dxf::Limits::DEFAULT).map(|_| ())
        };
        if r.is_ok() {
            oks += 1;
        } else {
            errs += 1;
        }
    }
    let ms = (plat::time_us() - t0) / 1000;
    t.ok(oks + errs == 600, &format!("600 damaged files (every third through the mapping): {oks} read, {errs} errors, no panic, {ms} ms"));
}

/// how far the GPU may be from the CPU path on a drawing: G6e1's loosest bounds of a category (the share of pixels beyond 2
/// codes of most categories, the largest difference of short segments; the ink of small text, which a drawing's labels
/// are: docs/RENDER2D.md "Der CPU-Weg")
pub const SHARE: f64 = 0.012;
pub const MAX: u8 = 210;
pub const INK: f64 = 1.0;

/// a scene of a drawing on the GPU and the CPU path: (pixels beyond 2 codes, largest, ink GPU against CPU %)
fn gpu_cpu(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    gfx::set_parallel(crate::cpucmp::par);
    let (fixture, r12) = (dxf_text(FIXTURE, "\r\n"), dxf_text(R12, "\n"));
    // (the drawings of ezdxf that are mostly text: the bounds of text, TEXT_SHARE)
    let files: [(&str, &[u8], bool); 5] = [("FIXTURE", fixture.as_bytes(), false), ("R12", r12.as_bytes(), false), ("ezdxf_clockwise_arcs_hatch", EZ_CW_HATCH, false),
                                           ("ezdxf_caret_encoding", EZ_CARET, true), ("ezdxf_mtext_complex_inline_codes", EZ_MTEXT, true)];
    for (name, bytes, text) in files {
        let (share, max, ink_max) = if text { (TEXT_SHARE, TEXT_MAX, TEXT_INK) } else { (SHARE, MAX, INK) };
        // (the drawings of ezdxf in the corpus test's 800 x 600: its bounds were set there; at 640 x 400 the 61 lines of
        // caret_encoding.dxf are small enough text for 4.4 % of the pixels to differ)
        let (w, h) = if name.starts_with("ezdxf") { (800u32, 600u32) } else { (640u32, 400u32) };
        let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
        let l = match render2d::dxf::load(&mut s, bytes, &options(BG, false)) {
            Ok(l) => l,
            Err(e) => {
                t.ok(false, &format!("{name}: {e}"));
                continue;
            }
        };
        let Some(ext) = l.extents else {
            t.ok(false, &format!("{name}: no extents"));
            continue;
        };
        let _ = s.set_camera(&fit(ext, (w, h)));
        let Some(mut off) = t.res(Off::new(&dev, w, h), "target") else { return };
        let Some(gimg) = shot(t, &dev, &mut off, &mut r, &mut s, name) else { continue };
        let cimg = cpu_image(&mut s, w, h, BG);
        crate::tests::dump(&format!("dxf_{name}_gpu"), &gimg, w, h);
        crate::tests::dump(&format!("dxf_{name}_cpu"), &cimg, w, h);
        let c = refimg::compare(&gimg, &cimg, w, BG, 2);
        let ink = 100.0 * (c.ink[0] - c.ink[1]) / c.ink[1].max(1.0);
        let px = (w * h) as f64;
        let inked = cimg.chunks_exact(4).filter(|p| (p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16) != BG & 0xFF_FFFF).count();
        t.ok(inked > 2000, &format!("{name}: the CPU path drew {inked} pixels"));
        t.ok((c.over as f64) <= share * px && c.max <= max && ink.abs() <= ink_max,
             &format!("{name}: the GPU against the CPU path: {} pixels beyond 2 codes ({:.3} %), largest {} at {:?}, ink {ink:+.3} % (at most {:.1} %, {max}, {ink_max} %)", c.over, 100.0 * c.over as f64 / px, c.max,
                      c.at, 100.0 * share));
    }
}

// ---------------------------------------------------------------------------------------------------- the corpus

/// where the corpus is: $JFLIB_DXF_CORPUS, else Documents/JFLib-dxf-korpus in the home folder
pub fn corpus_root() -> Option<String> {
    if let Some(d) = plat::env("JFLIB_DXF_CORPUS") {
        return Some(d);
    }
    let home = plat::env("USERPROFILE").or_else(|| plat::env("HOME"))?;
    Some(format!("{home}/Documents/JFLib-dxf-korpus"))
}

fn files_of(dir: &str) -> Vec<String> {
    let mut v = Vec::new();
    plat::dir_list(dir, &mut |name, is_dir, _| {
        if !is_dir && name.to_ascii_lowercase().ends_with(".dxf") {
            v.push(String::from(name));
        }
    });
    v.sort();
    v
}

/// a PNG of a BGRA image
fn write_png(path: &str, img: &[u8], w: u32, h: u32) -> bool {
    let mut sf = gfx::Surface::new(w as i32, h as i32, gfx::FMT_XRGB);
    for y in 0..h as usize {
        let row = &mut sf.px[y * sf.stride as usize..][..w as usize * 4];
        row.copy_from_slice(&img[y * w as usize * 4..][..w as usize * 4]);
    }
    match img_png::enc::encode_default(&sf) {
        Some(b) => plat::write_file(path, &b),
        None => false,
    }
}

/// the GPU against the CPU path on a drawing's scene: the comparison, the ink of the GPU against the CPU path in percent
/// (over at least half a code per pixel, so that an almost empty image gives no ratio of nothing), both images
fn gpu_against_cpu(t: &mut T, dev: &Device, r: &mut Renderer, s: &mut Scene, size: (u32, u32), what: &str) -> Option<(refimg::Cmp, f64, Vec<u8>, Vec<u8>)> {
    let cimg = cpu_image(s, size.0, size.1, BG);
    if !s.attached() {
        return None;
    }
    let mut off = Off::new(dev, size.0, size.1).ok()?;
    let gimg = shot(t, dev, &mut off, r, s, what)?;
    let c = refimg::compare(&gimg, &cimg, size.0, BG, 2);
    let floor = 0.5 * (size.0 * size.1) as f64;
    let ink = 100.0 * (c.ink[0] - c.ink[1]) / c.ink[1].max(floor);
    Some((c, ink, gimg, cimg))
}

/// how far the GPU may be from the CPU path where a drawing has text: small text (labels, about 8 to 12 px of cap height in
/// a whole drawing) is a few per cent darker from the distance fields than from its outlines (docs/RENDER2D.md "Der
/// CPU-Weg", the category Text), and text-heavy drawings show that over their whole ink
pub const TEXT_SHARE: f64 = 0.04;
pub const TEXT_INK: f64 = 8.0;
/// (the largest difference with text: none; class L of the atlas in blocks scaled up shows its known error, G6c)
pub const TEXT_MAX: u8 = 255;

/// render2dtest --test dxfcorpus
pub fn corpus(t: &mut T) {
    let Some(root) = corpus_root().filter(|r| plat::dir_list(r, &mut |_, _, _| {})) else {
        out("SKIP: no DXF corpus (JFLIB_DXF_CORPUS, or Documents/JFLib-dxf-korpus in the home folder)");
        return;
    };
    let png = plat::env("R2D_DXF_PNG");
    let gpu = setup(t).and_then(|g| g.adapter.create_device(&DeviceDesc::DEFAULT).ok());
    let mut r = gpu.as_ref().and_then(|d| Renderer::new(d, &RendererDesc::new(Format::Bgra8Unorm)).ok());
    gfx::set_parallel(crate::cpucmp::par);
    let size = (800u32, 600u32);
    let px = (size.0 * size.1) as f64;
    let mut worst: [(f64, String); 2] = [(0.0, String::new()), (0.0, String::new())];
    for src in ["gdal", "ezdxf", "dxf-parser", "qcad", "librecad"] {
        let dir = format!("{root}/{src}");
        let files = files_of(&dir);
        let (mut loaded, mut failed, mut mism) = (0u32, Vec::new(), 0u32);
        let mut over: [Vec<String>; 2] = [Vec::new(), Vec::new()];
        let mut kinds: alloc::collections::BTreeMap<String, render2d::dxf::Count> = alloc::collections::BTreeMap::new();
        for f in &files {
            let path = format!("{dir}/{f}");
            let Some(bytes) = plat::read_file(&path) else {
                t.ok(false, &format!("{src}/{f}: not readable"));
                continue;
            };
            let expect_err = dxf::is_binary(&bytes) || f.contains("fuzzer");
            let stem: String = f.trim_end_matches(".dxf").chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
            // twice: with the repository's fonts (the drawing as it is), without (no text: G6e1's bounds of the rest)
            for with_text in [true, false] {
                let mut s = match r.as_ref().map(Scene::new) {
                    Some(Ok(s)) => s,
                    _ => Scene::new_cpu(),
                };
                let mut o = options(BG, false);
                if !with_text {
                    o.fonts = None;
                }
                let l = match render2d::dxf::load(&mut s, &bytes, &o) {
                    Ok(l) => l,
                    Err(e) => {
                        if with_text {
                            if !expect_err {
                                t.ok(false, &format!("{src}/{f}: {e}"));
                            }
                            failed.push(format!("{f} ({})", match e {
                                dxf::Error::Binary => String::from("binary"),
                                dxf::Error::Invalid(s) | dxf::Error::Limit(s) => s,
                                other => format!("{other}"),
                            }));
                        }
                        break;
                    }
                };
                if with_text {
                    loaded += 1;
                    // the reader's kinds against the independent count
                    for ((wh, k), n) in markers(&bytes) {
                        let got = l.stats.count(wh, &k);
                        if got != n {
                            mism += 1;
                            t.ok(expect_err, &format!("{src}/{f}: {wh:?} {k}: the reader {got}, the markers {n}"));
                        }
                    }
                    for (k, c) in &l.kinds {
                        let e = kinds.entry(k.clone()).or_default();
                        e.read += c.read;
                        e.drawn += c.drawn;
                        e.paper += c.paper;
                        e.hidden += c.hidden;
                        e.unsupported += c.unsupported;
                        e.failed += c.failed;
                        e.approximated += c.approximated;
                    }
                }
                let Some(ext) = l.extents else { break };
                let _ = s.set_camera(&fit(ext, size));
                let k = if with_text { 0 } else { 1 };
                let (Some(dev), Some(rr)) = (gpu.as_ref(), r.as_mut()) else {
                    if let (Some(dir), true) = (&png, with_text) {
                        let cimg = cpu_image(&mut s, size.0, size.1, BG);
                        let _ = write_png(&format!("{dir}/{src}_{stem}.png"), &cimg, size.0, size.1);
                    }
                    continue;
                };
                let Some((c, ink, gimg, cimg)) = gpu_against_cpu(t, dev, rr, &mut s, size, f) else { continue };
                if let (Some(dir), true) = (&png, with_text) {
                    let _ = write_png(&format!("{dir}/{src}_{stem}.png"), &cimg, size.0, size.1);
                    let _ = write_png(&format!("{dir}/{src}_{stem}_gpu.png"), &gimg, size.0, size.1);
                }
                let share = c.over as f64 / px;
                if share > worst[k].0 {
                    worst[k] = (share, format!("{src}/{f}: {} pixels beyond 2 codes, largest {}, ink {ink:+.3} %", c.over, c.max));
                }
                let (bs, bm, bi) = if with_text { (TEXT_SHARE, TEXT_MAX, TEXT_INK) } else { (SHARE, MAX, INK) };
                if share > bs || c.max > bm || ink.abs() > bi {
                    over[k].push(format!("{f}: {:.3} % beyond 2 codes, largest {}, ink {ink:+.3} %", 100.0 * share, c.max));
                }
            }
        }
        t.ok(over[1].is_empty(), &format!("{src}: {loaded} of {} files loaded; without text the GPU against the CPU path within {:.1} % of the pixels, {MAX} codes and {INK} % of ink{}", files.len(),
                                          100.0 * SHARE, if over[1].is_empty() { String::new() } else { format!(" -- not: {}", over[1].join("; ")) }));
        t.ok(over[0].is_empty(), &format!("{src}: with text within {:.1} % of the pixels and {TEXT_INK} % of ink{}", 100.0 * TEXT_SHARE,
                                          if over[0].is_empty() { String::new() } else { format!(" -- not: {}", over[0].join("; ")) }));
        out(&format!("    {src}: not loaded: {}", if failed.is_empty() { String::from("none") } else { failed.join(", ") }));
        out(&format!("    {src}: {mism} kinds where the reader and the markers differ"));
        let mut line = String::new();
        for (k, c) in &kinds {
            line.push_str(&format!(" {k} {}", c.read));
            if c.drawn != c.read {
                line.push_str(&format!(" (drawn {}", c.drawn));
                for (n, v) in [("paper", c.paper), ("hidden", c.hidden), ("unsupported", c.unsupported), ("failed", c.failed)] {
                    if v > 0 {
                        line.push_str(&format!(", {n} {v}"));
                    }
                }
                line.push(')');
            }
            if c.approximated > 0 {
                line.push_str(&format!(" [approx. {}]", c.approximated));
            }
            line.push(';');
        }
        out(&format!("    {src}:{line}"));
    }
    out(&format!("    the largest difference GPU against CPU with text: {}", worst[0].1));
    out(&format!("    the largest difference GPU against CPU without text: {}", worst[1].1));
}

// ---------------------------------------------------------------------------------------------------- the viewer, shots

/// a drawing from a file into a scene, streamed (bounded memory), with the time it took and the peak of private bytes
pub fn load_file(s: &mut Scene, path: &str, o: &Options) -> core::result::Result<(Loaded, u64, u64), String> {
    let f = plat::File::open(path, plat::FO_READ).ok_or_else(|| format!("{path}: cannot open"))?;
    let mut at = 0u64;
    let mut peak = plat::win_test_usage().2;
    let mut next = 0u64;
    let mut src = |buf: &mut [u8]| {
        let n = f.read_at(at, buf).unwrap_or(0);
        at += n as u64;
        if at >= next {
            next = at + (16 << 20);
            peak = peak.max(plat::win_test_usage().2);
        }
        n
    };
    let t0 = plat::time_us();
    let l = render2d::dxf::read(s, &mut src, o).map_err(|e| format!("{path}: {e}"))?;
    let us = plat::time_us() - t0;
    peak = peak.max(plat::win_test_usage().2);
    Ok((l, us, peak))
}

/// render2dtest --dxf PATH: the demo's window on a drawing
pub fn viewer(path: &str) -> i32 {
    let mut s = Scene::new_cpu();
    let o = options(crate::demo::PAPER, true);
    // (":fixture": the drawing of --test dxf, for tests/dxf_view.uis)
    let got = if path == ":fixture" {
        let t0 = plat::time_us();
        render2d::dxf::load(&mut s, dxf_text(FIXTURE, "\r\n").as_bytes(), &o).map(|l| (l, plat::time_us() - t0, 0)).map_err(|e| format!("{e}"))
    } else {
        load_file(&mut s, path, &o)
    };
    let (l, us, _) = match got {
        Ok(v) => v,
        Err(e) => {
            plat::write_stderr(format!("render2dtest: {e}\n").as_bytes());
            return 1;
        }
    };
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let drawn: u64 = l.kinds.values().map(|c| c.drawn).sum();
    let unsupported: u64 = l.kinds.values().map(|c| c.unsupported).sum();
    let view = crate::demo::View2d::of(s);
    {
        let mut v = view.borrow_mut();
        v.fit = l.extents;
        v.title = Some(format!("{name}: {drawn} Entitäten ({unsupported} nicht unterstützt), {} ms", us / 1000));
        v.dxf = Some(l);
    }
    ui::launch(&crate::demo::DESC, crate::demo::Demo::with(view), &style::STYLE)
}

/// render2dtest --dxf-shot PATH OUT.png [--res WxH] [--gpu]
pub fn shot_file(path: &str, png: &str) -> i32 {
    let (w, h) = res(1600, 1200);
    let mut s = Scene::new_cpu();
    let (l, us, _) = match load_file(&mut s, path, &options(BG, false)) {
        Ok(v) => v,
        Err(e) => {
            out(&format!("FAIL {e}"));
            return 1;
        }
    };
    let Some(ext) = l.extents else {
        out("FAIL nothing drawn");
        return 1;
    };
    out(&format!("{path}: read and inserted in {} ms, {} render2d entities, extents {ext:?}", us / 1000, l.entities));
    // --view X0,Y0,X1,Y1: that box instead of the extents
    let ext = match plat::arg_value("--view").map(|v| v.split(',').map(|x| x.trim().parse::<f64>().unwrap_or(f64::NAN)).collect::<Vec<f64>>()) {
        Some(v) if v.len() == 4 && v.iter().all(|x| x.is_finite()) && v[2] > v[0] && v[3] > v[1] => [v[0], v[1], v[2], v[3]],
        _ => ext,
    };
    for (k, c) in &l.kinds {
        out(&format!("  {k}: {c:?}"));
    }
    for n in &l.notes {
        out(&format!("  note: {n}"));
    }
    let _ = s.set_camera(&fit(ext, (w, h)));
    gfx::set_parallel(crate::cpucmp::par);
    let img = cpu_image(&mut s, w, h, BG);
    if !write_png(png, &img, w, h) {
        out(&format!("FAIL cannot write {png}"));
        return 1;
    }
    0
}

fn res(dw: u32, dh: u32) -> (u32, u32) {
    match plat::arg_value("--res") {
        Some(v) if v.contains('x') => {
            let mut it = v.split('x').map(|x| x.parse::<u32>().unwrap_or(0));
            let (a, b) = (it.next().unwrap_or(dw), it.next().unwrap_or(dh));
            if a > 0 && b > 0 { (a, b) } else { (dw, dh) }
        }
        _ => (dw, dh),
    }
}

// ---------------------------------------------------------------------------------------------------- the bench

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// render2dtest --bench-dxf PATH: the reader alone, the reader into a scene, GPU frames (the scene's timestamps) and the
/// CPU path for the whole drawing and a zoomed view, memory (private bytes)
pub fn bench(path: &str) -> i32 {
    let size = res(1920, 1080);
    let frames = plat::arg_value("--frames").and_then(|v| v.parse::<usize>().ok()).unwrap_or(9).max(1);
    let mem0 = plat::win_test_usage().2;
    // the reader alone (a sink that only counts)
    {
        let Some(f) = plat::File::open(path, plat::FO_READ) else {
            out(&format!("FAIL {path}: cannot open"));
            return 1;
        };
        struct Null(u64);
        impl dxf::Sink for Null {
            fn entity(&mut self, _e: dxf::Entity, _: &dxf::Names) {
                self.0 += 1;
            }
        }
        let mut at = 0u64;
        let mut peak = mem0;
        let mut next = 0u64;
        let mut src = |buf: &mut [u8]| {
            let n = f.read_at(at, buf).unwrap_or(0);
            at += n as u64;
            if at >= next {
                next = at + (16 << 20);
                peak = peak.max(plat::win_test_usage().2);
            }
            n
        };
        let mut sink = Null(0);
        let t0 = plat::time_us();
        let r = dxf::read(&mut src, &dxf::Limits::DEFAULT, &mut sink);
        let ms = (plat::time_us() - t0) as f64 / 1000.0;
        match r {
            Ok((st, names)) => out(&format!("read (lib/dxf alone): {:.0} MB in {ms:.0} ms ({:.0} MB/s), {} lines, {} pairs, {} entities of ENTITIES, {} names; private bytes {:.0} MB more at the peak",
                                            st.bytes as f64 / 1e6, st.bytes as f64 / 1e6 / (ms / 1000.0).max(1e-9), st.lines, st.pairs, sink.0, names.len(), (peak.saturating_sub(mem0)) as f64 / 1e6)),
            Err(e) => {
                out(&format!("FAIL {e}"));
                return 1;
            }
        }
    }
    // into a scene without a device
    let mem1 = plat::win_test_usage().2;
    let mut s = Scene::new_cpu();
    let (l, us, peak) = match load_file(&mut s, path, &options(BG, false)) {
        Ok(v) => v,
        Err(e) => {
            out(&format!("FAIL {e}"));
            return 1;
        }
    };
    let after = plat::win_test_usage().2;
    out(&format!("read and inserted (render2d::dxf::read into Scene::new_cpu): {:.0} ms, {} render2d entities, {} layers in {} runs, {} blocks; private bytes {:.0} MB at the peak, {:.0} MB held after",
                 us as f64 / 1000.0, l.entities, l.layers.len(), l.layers.iter().map(|x| x.runs.len()).sum::<usize>(), l.blocks, peak.saturating_sub(mem1) as f64 / 1e6, after.saturating_sub(mem1) as f64 / 1e6));
    for (k, c) in &l.kinds {
        out(&format!("  {k}: read {}, drawn {}, failed {}, hidden {}, approximated {}", c.read, c.drawn, c.failed, c.hidden, c.approximated));
    }
    for n in &l.notes {
        out(&format!("  note: {n}"));
    }
    let Some(ext) = l.extents else {
        out("FAIL nothing drawn");
        return 1;
    };
    let full = fit(ext, size);
    // the zoomed view: --zoom F times at --zoom-at X,Y (else the middle of the extents)
    let zf = plat::arg_value("--zoom").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0).unwrap_or(40.0);
    let mut zoom = full;
    zoom.scale = full.scale / zf;
    if let Some(v) = plat::arg_value("--zoom-at") {
        let mut it = v.split(',').map(|x| x.trim().parse::<f64>().unwrap_or(f64::NAN));
        let (x, y) = (it.next().unwrap_or(f64::NAN), it.next().unwrap_or(f64::NAN));
        if x.is_finite() && y.is_finite() {
            zoom.center = [x, y];
        }
    }
    let zname = format!("zoomed {zf}x at ({:.0}, {:.0})", zoom.center[0], zoom.center[1]);
    let t0 = plat::time_us();
    let _ = s.set_camera(&full);
    let _ = s.build_index();
    out(&format!("the index (picking, the CPU path): {:.0} ms", (plat::time_us() - t0) as f64 / 1000.0));
    gfx::set_parallel(crate::cpucmp::par);
    for (name, cam) in [("whole", full), (zname.as_str(), zoom)] {
        let (ms1, _, info, img) = crate::cpucmp::bench_view(&mut s, &cam, size, 3.min(frames), None, BG);
        out(&format!("CPU path, {name}, {}x{}: {ms1:.1} ms per frame in row bands (median), {} entities found, {} drawn", size.0, size.1, info.found, info.drawn));
        if let Some(dir) = plat::env("R2D_DXF_PNG") {
            let stem = path.rsplit(['/', '\\']).next().unwrap_or("bench").trim_end_matches(".dxf");
            let _ = write_png(&format!("{dir}/bench_{stem}_{}.png", if name == "whole" { "whole" } else { "zoom" }), &img, size.0, size.1);
        }
    }
    // on the GPU
    let mut t = T { fails: 0, validate: false, inst: None };
    let Some(g) = setup(&mut t) else { return 0 };
    let Ok(dev) = g.adapter.create_device(&DeviceDesc::DEFAULT) else {
        out("SKIP: no device");
        return 0;
    };
    let Ok(mut r) = Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)) else {
        out("FAIL Renderer::new");
        return 1;
    };
    if s.attach(&r).is_err() {
        out("FAIL attach");
        return 1;
    }
    s.set_stats(true);
    // (G6e3: every failure says what failed; on the M2 the GPU part of Zürich ended without a word)
    let tex = match dev.create_texture(&TextureDesc::d2(size.0, size.1, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET)) {
        Ok(t) => t,
        Err(e) => {
            out(&format!("FAIL the target: {e}"));
            return 1;
        }
    };
    let mut used = false;
    let mut layers = l.clone();
    for (name, cam) in [("whole", full), (zname.as_str(), zoom)] {
        let _ = s.set_camera(&cam);
        let mut cpu_ms = Vec::new();
        let mut gpu_ms = Vec::new();
        let mut wall_ms = Vec::new();
        let mut first = 0.0;
        for k in 0..frames + 1 {
            let t0 = plat::time_us();
            if let Err(e) = s.prepare(&mut r, size) {
                out(&format!("FAIL prepare: {e}"));
                return 1;
            }
            let prep = (plat::time_us() - t0) as f64 / 1000.0;
            let mut e = match dev.encoder() {
                Ok(e) => e,
                Err(e) => {
                    out(&format!("FAIL encoder: {e}"));
                    return 1;
                }
            };
            let color = ColorAttachment { view: tex.view(), load: Load::Clear, clear: bg_clear(BG), store: true, before: if used { Layout::ColorTarget } else { Layout::Undefined },
                                          after: Layout::ColorTarget };
            if let Err(err) = r.record(&mut e, &s, color) {
                out(&format!("FAIL record: {err}"));
                return 1;
            }
            let cmd = match e.finish() {
                Ok(c) => c,
                Err(err) => {
                    out(&format!("FAIL finish: {err}"));
                    return 1;
                }
            };
            // (G6e3) the wall time from the submission until the wait came back: what Metal (no timestamps in lib/gpu) says
            let t1 = plat::time_us();
            let id = match dev.queue().submit([cmd]) {
                Ok(id) => id,
                Err(err) => {
                    out(&format!("FAIL submit: {err}"));
                    return 1;
                }
            };
            s.submitted(id);
            used = true;
            if !dev.wait(id, 60_000_000_000).unwrap_or(false) {
                out("FAIL a frame did not finish within 60 s");
                return 1;
            }
            let wall = (plat::time_us() - t1) as f64 / 1000.0;
            s.collect_stats();
            let st = s.take_stats();
            let gms = st.last().and_then(|f| f.stamps).map_or(0.0, |v| v[3] - v[0]);
            if k == 0 {
                first = prep;
            } else {
                cpu_ms.push(prep);
                gpu_ms.push(gms);
                wall_ms.push(wall);
            }
        }
        out(&format!("GPU, {name}, {}x{}: {:.2} ms per frame on the GPU (median of {frames}), prepare {:.2} ms (the first frame's: {first:.0} ms), submit to done {:.2} ms (wall)", size.0,
                     size.1, median(&mut gpu_ms), median(&mut cpu_ms), median(&mut wall_ms)));
    }
    // --per-layer: the whole view's GPU time with each DXF layer hidden in turn (what each costs)
    if plat::args().iter().any(|a| a == "--per-layer") {
        let _ = s.set_camera(&full);
        let names: Vec<String> = layers.layers.iter().map(|x| x.name.clone()).collect();
        for n in names {
            let _ = layers.set_layer_visible(&mut s, &n, false);
            let mut gpu_ms = Vec::new();
            for _ in 0..3 {
                if s.prepare(&mut r, size).is_err() {
                    return 1;
                }
                let Ok(mut e) = dev.encoder() else { return 1 };
                let color = ColorAttachment { view: tex.view(), load: Load::Clear, clear: bg_clear(BG), store: true, before: Layout::ColorTarget, after: Layout::ColorTarget };
                if r.record(&mut e, &s, color).is_err() {
                    return 1;
                }
                let Ok(cmd) = e.finish() else { return 1 };
                let Ok(id) = dev.queue().submit([cmd]) else { return 1 };
                s.submitted(id);
                let _ = dev.wait(id, 60_000_000_000);
                s.collect_stats();
                let st = s.take_stats();
                gpu_ms.push(st.last().and_then(|f| f.stamps).map_or(0.0, |v| v[3] - v[0]));
            }
            out(&format!("  without {n}: {:.1} ms", median(&mut gpu_ms)));
            let _ = layers.set_layer_visible(&mut s, &n, true);
        }
    }
    0
}
