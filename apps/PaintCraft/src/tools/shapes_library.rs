// shapes_lib -- PaintCraft: shape library of the Formen tool (agent T4).  Built-in shapes
// (paint.net 5.1 list and German names; geometry = path data in a 0..100 box, parsed with
// gfx/pathdata.s) and custom shapes from paint.net ShapeKit XAML files
// (<ps:SimpleGeometryShape DisplayName="…" Geometry="…"/>, or PathGeometry Figures="…" inside
// the element) in <exe folder>/Shapes.  Docs: apps/PaintCraft/docs/SHAPES.md
//                                              [apps/PaintCraft/src/shapes_lib.s]
//
//   shl_init()                        parse the built-in shapes, load <exe dir>/Shapes/*.xaml
//   shl_count() -> shapes             (built-in first)
//   shl_name(i) -> German / display name
//   shl_radius(i) -> true when the shape uses the corner radius (Ecke Größe)
//   shl_emit(i, &box, &M, radius, path) -> 0 / ERR_FAIL
//                                     path += shape i fitted into box {x0, y0, x1, y1} (any
//                                     corner order = mirrored), then the affine M; radius for
//                                     the rounded rectangle (box units)
//   shl_rule(i) -> RULE_NONZERO / RULE_EVENODD
//   shl_load_dir(path) -> custom shapes loaded (replaces the custom shapes by the *.xaml files
//                                     of path)
//   shl_status() -> (text, failed files)   "N eigene Formen" + the files that failed
//   shl_gallery(cur) -> the chosen shape or -1   tool row: "Art der Form" drop-down
//                                     (ID_SHAPE_LIST) with the gallery flyout (cells
//                                     ID_SHAPE_LIST + 1 + i)
//   shl_add_xaml(buf, name) -> shapes added   (P1) custom shapes from an in-memory XAML
//                                     document (plugin shapes; shl_load_dir re-adds them
//                                     through plug_shapes_feed after every folder load)
//   shl_draw_icon(user, rect)         outline of shape (user & 0xFFFF) in the colour user >> 32
//                                     inside rect {x, y, w, h} (px); the cmd_custom callback
//                                     of the gallery cells and the button (ShlIcon)
//
// Rust model: the shape records are Rc'd (the render-time icons keep their record); the status
// text is rebuilt whenever it changes (shl_status borrows it).

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

use gfx::path::Path;
use gfx::pathdata::PData;
use gfx::xform::Matrix;
use gfx::{CAP_ROUND, JOIN_MITER, RULE_EVENODD};
use img_core::xform::mul;
use ui::*;

use crate::app::constants::*;
use crate::app::Pn;

const SHC_BASIC: u32 = 0;
const SHC_POLY: u32 = 1;
const SHC_ARROWS: u32 = 2;
const SHC_CALLOUTS: u32 = 3;
const SHC_SYMBOLS: u32 = 4;
const SHC_CUSTOM: u32 = 5;
const SHC_COUNT: u32 = 6;
const SHK_PATH: u32 = 0;
const SHK_RRECT: u32 = 1;
const SH_MAX: usize = 256;
const NAME_CAP: usize = 64; // custom names (bytes incl. NUL)
const FILES_MAX: usize = 128;
const FNAME_CAP: usize = 128;
const PATH_CAP: usize = 512;
const XAML_MAX: usize = 0x100000; // 1 MB per file
const ERR_CAP: usize = 512;
const GAL_COLS: i32 = 8;
const CELL_DIP: i32 = 28;

/// a shape record (SH_*)
pub(crate) struct ShapeRec {
    /// SH_PD: the geometry
    pd: PData,
    /// SH_B: f64 bounds x0, y0, x1, y1 of the geometry
    b: [f64; 4],
    /// SH_NAME
    name: String,
    /// SH_CAT: SHC_*
    cat: u32,
    /// SH_KIND: SHK_*
    kind: u32,
}

/// the render-time icon of a shape (cmd_custom: shl_draw_icon); one per shape, so the
/// command's identity stays the same from frame to frame
pub(crate) struct ShlIcon {
    rec: Rc<ShapeRec>,
    dpi: Cell<i32>,
    argb: Cell<u32>,
}

impl Draw for ShlIcon {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        draw_icon(&self.rec, g, self.argb.get(), self.dpi.get(), r);
    }
}

/// the state of shapes_lib.s (its g_* globals)
#[derive(Default)]
pub(crate) struct Shl {
    /// g_shl: the shape records (built-in first)
    shapes: Vec<Rc<ShapeRec>>,
    /// built-in shapes, custom shapes, files that failed
    nbi: usize,
    ncust: i32,
    nfail: i32,
    /// files that failed, ", " separated
    err: String,
    /// shl_status text
    stat: String,
    /// shape shown on the gallery button
    btncur: i32,
    /// the icons of the shapes (rebuilt with the records)
    icons: Vec<Rc<ShlIcon>>,
}

// shl_rec(i) -> record (i clamped to the table)
fn shl_rec(s: &Shl, i: i32) -> &ShapeRec {
    let k = if (i as u32 as usize) < s.shapes.len() { i as usize } else { 0 };
    &s.shapes[k & (SH_MAX - 1)]
}

/// shl_count() -> shapes
pub(crate) fn shl_count(s: &Shl) -> i32 {
    s.shapes.len() as i32
}

/// shl_name(i)
pub(crate) fn shl_name(s: &Shl, i: i32) -> &str {
    if s.shapes.is_empty() {
        return "";
    }
    &shl_rec(s, i).name
}

/// shl_radius(i) -> 1 for the rounded rectangle
pub(crate) fn shl_radius(s: &Shl, i: i32) -> bool {
    !s.shapes.is_empty() && shl_rec(s, i).kind == SHK_RRECT
}

/// shl_rule(i) -> fill rule
pub(crate) fn shl_rule(s: &Shl, i: i32) -> u32 {
    if s.shapes.is_empty() {
        return 0;
    }
    shl_rec(s, i).pd.rule
}

/// shl_emit(i, &box, &M, radius) -> 0 / ERR_FAIL -- the shape's outline into `path`
pub(crate) fn shl_emit(s: &Shl, i: i32, bx: &[f64; 4], m: &Matrix, radius: f64, path: &mut Path) -> i32 {
    if s.shapes.is_empty() {
        return 0;
    }
    emit_rec(shl_rec(s, i), bx, m, radius, path)
}

// shl_emit of one record
fn emit_rec(rec: &ShapeRec, bx: &[f64; 4], m: &Matrix, radius: f64, path: &mut Path) -> i32 {
    if rec.kind == SHK_RRECT {
        let mut pd = PData::new();
        pd.rrect(&[bx[0], bx[1], bx[2], bx[3], radius]);
        return pd.emit(path, Some(m));
    }
    // S: x' = box.x0 + (x - b.x0) * sx, sx = (box.x1 - box.x0) / (b.x1 - b.x0)
    let mut sm = Matrix::IDENTITY;
    for axis in 0..2 {
        let ge = rec.b[axis + 2] - rec.b[axis]; // geometry extent
        let be = bx[axis + 2] - bx[axis]; // box extent
        let sc = if ge < 0.0 || ge > 0.0 { be / ge } else { 0.0 };
        let t = bx[axis] - rec.b[axis] * sc;
        if axis == 0 {
            sm.a = sc;
            sm.c = t;
        } else {
            sm.e = sc;
            sm.f = t;
        }
    }
    let t = mul(m, &sm);
    rec.pd.emit(path, Some(&t))
}

// ------------------------------------------------------------------ initialisation

// cut(s, cap) -- str_copy: at most cap - 1 bytes, never splitting a UTF-8 sequence
fn cut(s: &str, cap: usize) -> &str {
    if s.len() < cap {
        return s;
    }
    let mut n = cap - 1;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

// str_cat(dst, src, cap): append, the whole text cut like str_copy
fn cat(dst: &mut String, src: &str, cap: usize) {
    if dst.len() + 1 >= cap {
        return;
    }
    let room = cap - dst.len();
    dst.push_str(cut(src, room));
}

/// shl_init() -- the built-in shapes, then <exe dir>/Shapes
pub(crate) fn shl_init(p: &mut Pn) {
    let s = &mut p.shl;
    s.shapes.clear();
    s.nbi = 0;
    shl_free_custom(s);
    for &(name, data, cat, kind) in BUILTIN.iter() {
        if s.shapes.len() >= SH_MAX {
            break;
        }
        let mut pd = PData::new();
        let b = if kind == SHK_RRECT {
            [0.0, 0.0, 100.0, 100.0]
        } else {
            pd.parse(data.as_bytes());
            pd.bounds_box(None).1
        };
        s.shapes.push(Rc::new(ShapeRec { pd, b, name: String::from(name), cat, kind }));
    }
    s.nbi = s.shapes.len();
    s.icons.clear();
    // <exe dir>/Shapes (argv[0] up to its last separator; no separator: ./Shapes)
    let a0 = plat::args().into_iter().next().unwrap_or_default();
    let a0 = cut(&a0, PATH_CAP - 16);
    let path = match a0.bytes().take(PATH_CAP - 16).rposition(|c| c == b'/' || c == b'\\') {
        Some(k) => {
            let mut s = String::from(&a0[..k + 1]);
            s.push_str("Shapes");
            s
        }
        None => String::from("Shapes"),
    };
    shl_load_dir(p, &path);
    crate::integrations::paint_dot_net::catalog::shapes_feed(p,&path);
}

// shl_free_custom() -- drop the custom shapes and the error text
fn shl_free_custom(s: &mut Shl) {
    let nbi = s.nbi;
    s.shapes.truncate(nbi);
    s.icons.truncate(nbi);
    s.nfail = 0;
    s.ncust = 0;
    s.err.clear();
    shl_stat_update(s);
}

/// shl_load_dir(path) -> custom shapes loaded
pub(crate) fn shl_load_dir(p: &mut Pn, path: &str) -> i32 {
    shl_free_custom(&mut p.shl);
    let dir = String::from(cut(path, PATH_CAP - FNAME_CAP - 8));
    // the *.xaml names (sorted: the directory order differs between systems)
    let mut files: Vec<String> = Vec::new();
    plat::dir_list(&dir, &mut |name, is_dir, _| {
        if is_dir || !str_ends_ci(name, ".xaml") || files.len() >= FILES_MAX || name.len() >= FNAME_CAP - 1 {
            return;
        }
        files.push(String::from(name));
    });
    files.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    for name in &files {
        // full path = dir "/" name
        let mut f = String::from(cut(&dir, PATH_CAP));
        cat(&mut f, "/", PATH_CAP);
        cat(&mut f, name, PATH_CAP);
        shl_load_file(p, &f, name);
    }
    crate::integrations::plugins::plug_shapes_feed(p); // P1: the shapes of loaded plugins
    let s = &mut p.shl;
    s.ncust = (s.shapes.len() - s.nbi) as i32;
    shl_stat_update(s);
    s.ncust
}

// str_ends(s, suffix): ASCII case-insensitive
fn str_ends_ci(s: &str, suf: &str) -> bool {
    let (a, b) = (s.as_bytes(), suf.as_bytes());
    a.len() >= b.len() && a[a.len() - b.len()..].eq_ignore_ascii_case(b)
}

/// (P1) shl_add_xaml(buf, len, name) -> shapes added -- the shapes of an in-memory ShapeKit
/// XAML document (all or none, like one file; failures go to shl_status)
pub(crate) fn shl_add_xaml(p: &mut Pn, buf: &[u8], name: &str) -> i32 {
    let s = &mut p.shl;
    let before = s.shapes.len();
    let mut r = shl_parse_xaml(s, buf, name);
    if r <= 0 {
        s.shapes.truncate(before);
        shl_fail(s, name);
        r = 0;
    }
    s.ncust = (s.shapes.len() - s.nbi) as i32;
    shl_stat_update(s);
    r
}

// shl_fail(name) -- remember a file that could not be loaded (", name" appended)
fn shl_fail(s: &mut Shl, name: &str) {
    s.nfail += 1;
    if !s.err.is_empty() {
        cat(&mut s.err, ", ", ERR_CAP);
    }
    cat(&mut s.err, name, ERR_CAP);
    shl_stat_update(s);
}

// shl_load_file(path, name) -- the shapes of one XAML file (all or none)
fn shl_load_file(p: &mut Pn, path: &str, name: &str) {
    let s = &mut p.shl;
    let before = s.shapes.len();
    if let Some(data) = plat::read_file(path) {
        if data.len() <= XAML_MAX && shl_parse_xaml(s, &data, name) > 0 {
            return;
        }
        s.shapes.truncate(before); // drop what this file added
    }
    shl_fail(s, name);
}

// shl_find(buf, p, end, needle) -> first occurrence of needle in [p, end)
fn shl_find(buf: &[u8], p: usize, end: usize, needle: &[u8]) -> Option<usize> {
    let n = needle.len();
    let mut i = p;
    while end.saturating_sub(i) >= n && i <= end {
        if &buf[i..i + n] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

// shl_isname(c) -> 1 for XML name characters [A-Za-z0-9_:.-]
fn shl_isname(c: u8) -> bool {
    c.is_ascii_digit() || (c | 32).wrapping_sub(b'a') <= 25 || matches!(c, b'_' | b':' | b'.' | b'-')
}

// shl_isws(c) -> 1 for XML white space
fn shl_isws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

// shl_attr(buf, p, end, name) -> (value start, value length) -- the attribute name="value"
// (or 'value') in [p, end): the name preceded by white space, followed by optional blanks,
// '=', optional blanks and a quote; the value ends at the same quote
fn shl_attr(buf: &[u8], mut p: usize, end: usize, name: &[u8]) -> Option<(usize, usize)> {
    loop {
        let hit = shl_find(buf, p, end, name)?;
        p = hit + 1; // next search after this hit
        // preceded by white space
        if hit == 0 || !shl_isws(buf[hit - 1]) {
            continue;
        }
        let mut i = hit + name.len();
        loop {
            if i >= end {
                return None;
            }
            if !shl_isws(buf[i]) {
                break;
            }
            i += 1;
        }
        if buf[i] != b'=' {
            continue;
        }
        i += 1;
        loop {
            if i >= end {
                return None;
            }
            if !shl_isws(buf[i]) {
                break;
            }
            i += 1;
        }
        let q = buf[i];
        if q != b'"' && q != b'\'' {
            continue;
        }
        i += 1;
        let mut e = i;
        loop {
            if e >= end {
                return None;
            }
            if buf[e] == q {
                return Some((i, e - i));
            }
            e += 1;
        }
    }
}

static ENTS: [(&[u8], u32); 5] = [(b"amp", 38), (b"lt", 60), (b"gt", 62), (b"quot", 34), (b"apos", 39)];

// shl_entity(buf, p, end) -> (code point, position after the ';') or None
fn shl_entity(buf: &[u8], p: usize, end: usize) -> Option<(u32, usize)> {
    let mut r = p + 1;
    if r >= end {
        return None;
    }
    if buf[r] != b'#' {
        for &(name, cp) in ENTS.iter() {
            let e = r + name.len();
            if e >= end || buf[e] != b';' {
                continue;
            }
            if &buf[r..e] == name {
                return Some((cp, e + 1));
            }
        }
        return None;
    }
    r += 1;
    let mut base = 10u32;
    if r >= end {
        return None;
    }
    if buf[r] | 32 == b'x' {
        base = 16;
        r += 1;
    }
    let mut v = 0u32;
    let mut digits = 0;
    loop {
        if r >= end {
            return None;
        }
        let c = buf[r] as u32;
        if c == b';' as u32 {
            break;
        }
        let mut d = c.wrapping_sub(48);
        if d > 9 {
            if base != 16 {
                return None;
            }
            d = (c | 32).wrapping_sub(87);
            if !(10..=15).contains(&d) {
                return None;
            }
        }
        v = v * base + d;
        if v > 0x10FFFF {
            return None;
        }
        digits += 1;
        r += 1;
    }
    if digits == 0 || v == 0 {
        return None;
    }
    Some((v, r + 1))
}

// utf8_encode(cp) -> bytes (code points above U+10FFFF become U+FFFD)
fn utf8_encode(cp: u32, out: &mut Vec<u8>) {
    let cp = if cp > 0x10FFFF { 0xFFFD } else { cp };
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | (cp >> 6) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else if cp < 0x10000 {
        out.push(0xE0 | (cp >> 12) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else {
        out.push(0xF0 | (cp >> 18) as u8);
        out.push(0x80 | ((cp >> 12) & 0x3F) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    }
}

// shl_decode(cap, src) -- XML text -> UTF-8 (&amp; &lt; &gt; &quot; &apos; &#N; &#xH;), cut to
// cap - 1 bytes
fn shl_decode(buf: &[u8], p: usize, n: usize, cap: usize) -> String {
    let mut out: Vec<u8> = Vec::new();
    let mut i = p;
    let end = p + n;
    let mut tmp = Vec::new();
    while i < end {
        let c = buf[i];
        if c == b'&' {
            if let Some((cp, next)) = shl_entity(buf, i, end) {
                i = next;
                tmp.clear();
                utf8_encode(cp, &mut tmp);
                for &b in tmp.iter().take(4) {
                    if out.len() < cap - 1 {
                        out.push(b);
                    }
                }
                continue;
            }
        }
        i += 1;
        if out.len() < cap - 1 {
            out.push(c);
        }
    }
    match String::from_utf8(out) {
        Ok(s) => s,
        Err(e) => String::from(String::from_utf8_lossy(e.as_bytes()).as_ref()),
    }
}

// shl_parse_xaml(buf, fname) -> shapes added (0 / -1 = the file fails); the new records are
// appended to s.shapes (the caller rolls back)
fn shl_parse_xaml(s: &mut Shl, buf: &[u8], fname: &str) -> i32 {
    const SGS: &[u8] = b"SimpleGeometryShape";
    const SGS_END: &[u8] = b"SimpleGeometryShape>";
    let end = buf.len();
    let mut p = 0usize;
    let mut count = 0;
    'next: loop {
        let Some(hit) = shl_find(buf, p, end, SGS) else { return count };
        p = hit + 19;
        let ne = hit + 19; // element name end
        // an element start: '<' [prefix:] SimpleGeometryShape (white space | '/' | '>')
        let mut i = hit;
        loop {
            if i == 0 {
                continue 'next;
            }
            let c = buf[i - 1];
            if c == b'<' {
                break;
            }
            if c != b':' && !shl_isname(c) {
                continue 'next;
            }
            i -= 1;
        }
        if ne >= end {
            return -1;
        }
        let c = buf[ne];
        if c != b'/' && c != b'>' && !shl_isws(c) {
            continue; // e.g. SimpleGeometryShape.Geometry
        }
        // the tag end '>' (quoted values skipped)
        let mut te = ne;
        let mut q = 0u8;
        loop {
            if te >= end {
                return -1;
            }
            let c = buf[te];
            if q != 0 {
                if c == q {
                    q = 0;
                }
            } else if c == b'>' {
                break;
            } else if c == b'"' || c == b'\'' {
                q = c;
            }
            te += 1;
        }
        // a new record
        if s.shapes.len() >= SH_MAX {
            return -1;
        }
        let mut pd = PData::new();
        // DisplayName (else the file name without its extension)
        let name = match shl_attr(buf, ne, te, b"DisplayName") {
            Some((a, n)) => shl_decode(buf, a, n, NAME_CAP),
            None => {
                let f = cut(fname, NAME_CAP);
                String::from(match f.rfind('.') {
                    Some(k) => &f[..k],
                    None => f,
                })
            }
        };
        if let Some((a, n)) = shl_attr(buf, ne, te, b"Geometry") {
            if pd.parse(&buf[a..a + n]).0 != 0 {
                return -1;
            }
            p = te + 1;
        } else {
            // <…SimpleGeometryShape …> … Figures="…" … </…SimpleGeometryShape>
            if buf[te - 1] == b'/' {
                return -1; // self-closing without Geometry
            }
            let Some(ce) = shl_find(buf, te, end, SGS_END) else { return -1 };
            p = ce + 20;
            let mut sp = te;
            let mut figs = 0;
            while let Some((a, n)) = shl_attr(buf, sp, ce, b"Figures") {
                sp = a + n;
                if pd.parse(&buf[a..a + n]).0 != 0 {
                    return -1;
                }
                figs += 1;
            }
            if figs == 0 {
                return -1;
            }
            // FillRule="EvenOdd" anywhere inside
            if let Some((a, 7)) = shl_attr(buf, te, ce, b"FillRule") {
                let w = u32::from_le_bytes([buf[a], buf[a + 1], buf[a + 2], buf[a + 3]]) | 0x2020_2020;
                if w == 0x6E65_7665 {
                    pd.rule = RULE_EVENODD;
                }
            }
        }
        if pd.ncmd == 0 {
            return -1;
        }
        let Some(b) = pd.bounds(None) else { return -1 };
        // an outline with an area: width and height > 0
        if !(b[2] - b[0] > 0.0) || !(b[3] - b[1] > 0.0) {
            return -1;
        }
        s.shapes.push(Rc::new(ShapeRec { pd, b, name, cat: SHC_CUSTOM, kind: SHK_PATH }));
        count += 1;
    }
}

// shl_status text: "%d eigene Formen" (+ "; nicht geladen: " + the files)
fn shl_stat_update(s: &mut Shl) {
    let mut t = alloc::format!("{} eigene Formen", s.ncust);
    if s.nfail != 0 {
        cat(&mut t, "; nicht geladen: ", ERR_CAP + 64);
        cat(&mut t, &s.err, ERR_CAP + 64);
    }
    s.stat = t;
}

/// shl_status() -> (text, failed files)
pub(crate) fn shl_status(s: &Shl) -> (&str, i32) {
    (&s.stat, s.nfail)
}

// ------------------------------------------------------------------ gallery

// draw_icon(rec, argb, dpi, rect): the outline of the shape inside rect (render time)
fn draw_icon(rec: &ShapeRec, g: &mut Gfx, argb: u32, dpi: i32, r: [i32; 4]) {
    g.dc.set_color(argb);
    let inset = dp_at(4, dpi);
    let bx = [
        (r[0] + inset) as f64 + 0.5,
        (r[1] + inset) as f64 + 0.5,
        (r[0] + r[2] - inset) as f64 - 0.5,
        (r[1] + r[3] - inset) as f64 - 0.5,
    ];
    let radius = dp_at(4, dpi) as f64;
    let Gfx { dc, sf, text } = g;
    let path = &mut text.path;
    path.begin(dc);
    emit_rec(rec, &bx, &Matrix::IDENTITY, radius, path);
    // 1 px at 100 %, scaled with the dpi
    let w = ((dpi as u32) << 8) / 96;
    path.stroke(&mut text.ras, dc, sf, w as i32, JOIN_MITER | CAP_ROUND << 4);
}

/// shl_draw_icon(user, rect) -- cmd_custom callback (render time, UI thread)
pub(crate) fn shl_draw_icon(s: &Shl, g: &mut Gfx, user: usize, r: [i32; 4]) {
    if s.shapes.is_empty() {
        return;
    }
    let dpi = s.icons.first().map_or(96, |i| i.dpi.get());
    #[cfg(target_pointer_width = "64")]
    let color = (user >> 32) as u32;
    #[cfg(target_pointer_width = "32")]
    let color = s.icons.get(user & 0xFFFF).map_or(0xff000000, |icon| icon.argb.get());
    draw_icon(shl_rec(s, (user & 0xFFFF) as i32), g, color, dpi, r);
}

// Keep the original native change key; wasm32 splits color into the version field.
fn draw_key(shape: i32, color: u32) -> (usize, u32) {
    #[cfg(target_pointer_width = "64")]
    { ((shape as u32 as usize) | (color as usize) << 32, 0) }
    #[cfg(target_pointer_width = "32")]
    { (shape as u32 as usize, color) }
}

// shl_icon(i, argb, dpi) -> the icon of shape i (created once per record)
fn shl_icon(s: &mut Shl, i: i32, argb: u32, dpi: i32) -> Rc<ShlIcon> {
    let k = if (i as u32 as usize) < s.shapes.len() { i as usize } else { 0 } & (SH_MAX - 1);
    while s.icons.len() < s.shapes.len() {
        let rec = s.shapes[s.icons.len()].clone();
        s.icons.push(Rc::new(ShlIcon { rec, dpi: Cell::new(dpi), argb: Cell::new(argb) }));
    }
    let ic = s.icons[k].clone();
    ic.dpi.set(dpi);
    ic.argb.set(argb);
    ic
}

// shl_cell(i, cur) -> EV -- one gallery cell
fn shl_cell(p: &mut Pn, ui: &mut Ui, i: i32, cur: i32) -> u32 {
    let d = ui.dp(CELL_DIP);
    ui.place(d, d);
    let id = ID_SHAPE_LIST + 1 + i as u32;
    let ev = ui.behave(id, 0);
    // background: accent frame for the current shape, subtle fill while hovered
    let fill = if ev & EV_HOVER != 0 { ui.color(T_SUBTLE_HOVER) } else { 0 };
    let stroke = if i == cur { ui.color(T_ACCENT) } else { 0 };
    let rad = ui.dp(4);
    let border = if stroke != 0 { ui.dp(2) } else { 0 };
    let r = ui.last_rect();
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, border, fill, stroke, stroke);
    let tc = ui.color(T_TEXT);
    let dpi = ui.dpi();
    let ic = shl_icon(&mut p.shl, i, tc, dpi);
    let f: DrawRef = ic;
    let (user, version) = draw_key(i, tc);
    ui.cmd_custom(r[0], r[1], r[2], r[3], Some(f), user, version);
    let name = String::from(shl_name(&p.shl, i));
    let sem = ui.sem(K_BUTTON, id, Some(&name), 0);
    if i == cur {
        sem.state |= SS_CHECKED;
    }
    ui.tooltip(&name);
    ev
}

// shl_btn_icon() -- draw shape btncur into the left square of the last widget (the gallery
// button)
fn shl_btn_icon(p: &mut Pn, ui: &mut Ui) {
    let cur = p.shl.btncur;
    let r = ui.last_rect();
    let inset = ui.dp(4);
    let tc = ui.color(T_TEXT);
    let dpi = ui.dpi();
    let i = cur & 0xFFFF;
    let ic = shl_icon(&mut p.shl, i, tc, dpi);
    let f: DrawRef = ic;
    let (user, version) = draw_key(i, tc);
    ui.cmd_custom(r[0] + inset, r[1], r[3], r[3], Some(f), user, version);
}

static CATNAMES: [&str; 6] = ["Grundlegende Formen", "Polygone und Sterne", "Pfeile", "Sprechblasen", "Symbole", "Eigene Formen"];

/// shl_gallery(cur) -> chosen shape or -1
pub(crate) fn shl_gallery(p: &mut Pn, ui: &mut Ui, cur: i32) -> i32 {
    p.shl.btncur = cur;
    let mut chosen = -1;
    // the button: blank label (room for the icon of the current shape) + chevron
    ui.next_flags(ST_SUBTLE);
    ui.dropdown_button(ID_SHAPE_LIST, crate::preferences::localization::tr("       "));
    shl_btn_icon(p, ui);
    let name = String::from(shl_name(&p.shl, cur));
    ui.tooltip(&name);
    if ui.flyout_begin(ID_SHAPE_LIST, FL_BELOW) {
        for c in 0..SHC_COUNT {
            // any shape of this category?
            if !p.shl.shapes.iter().any(|r| r.cat == c) {
                continue;
            }
            ui.text(TS_BODY_STRONG, CATNAMES[c as usize]);
            ui.box_begin(0, BOX_H, 2);
            let mut col = 0;
            for i in 0..p.shl.shapes.len() {
                if p.shl.shapes[i].cat != c {
                    continue;
                }
                if col >= GAL_COLS {
                    ui.box_end();
                    ui.box_begin(0, BOX_H, 2);
                    col = 0;
                }
                if shl_cell(p, ui, i as i32, cur) & EV_ACT != 0 {
                    chosen = i as i32;
                    ui.popup_close();
                }
                col += 1;
            }
            ui.box_end();
        }
        if p.shl.nfail != 0 {
            let st = String::from(&p.shl.stat);
            ui.text(TS_CAPTION, &st);
        }
    }
    ui.flyout_end();
    chosen
}

// ---- built-in shapes (geometry ported from apps/paint/src/shapes_geom.s, the rest drawn for
// the paint.net 5.1 list; path data in a 0..100 box, y down)
static BUILTIN: [(&str, &str, u32, u32); 30] = [
    ("Rechteck", "M0 0H100V100H0Z", SHC_BASIC, SHK_PATH),
    ("Abgerundetes Rechteck", "", SHC_BASIC, SHK_RRECT),
    ("Ellipse", "M100 50A50 50 0 1 1 0 50A50 50 0 1 1 100 50Z", SHC_BASIC, SHK_PATH),
    ("Raute", "M50 0L100 50L50 100L0 50Z", SHC_BASIC, SHK_PATH),
    ("Parallelogramm", "M25 0H100L75 100H0Z", SHC_BASIC, SHK_PATH),
    ("Trapez", "M25 0H75L100 100H0Z", SHC_BASIC, SHK_PATH),
    ("Dreieck", "M50 0L100 100H0Z", SHC_BASIC, SHK_PATH),
    ("Rechtwinkliges Dreieck", "M0 0L100 100H0Z", SHC_BASIC, SHK_PATH),
    ("Fünfeck", "M50 0L97.55 34.55L79.39 90.45L20.61 90.45L2.45 34.55Z", SHC_POLY, SHK_PATH),
    ("Sechseck", "M0 50L25 0L75 0L100 50L75 100L25 100Z", SHC_POLY, SHK_PATH),
    ("Heptagon", "M50 0L89.09 18.83L98.75 61.13L71.69 95.05L28.31 95.05L1.25 61.13L10.91 18.83Z", SHC_POLY, SHK_PATH),
    ("Achteck", "M69.13 3.81L96.19 30.87L96.19 69.13L69.13 96.19L30.87 96.19L3.81 69.13L3.81 30.87L30.87 3.81Z", SHC_POLY, SHK_PATH),
    ("Stern (3 Zacken)", "M50 0L62.99 42.5L93.3 75L50 65L6.7 75L37.01 42.5Z", SHC_POLY, SHK_PATH),
    ("Stern (4 Zacken)", "M50 0L63.44 36.56L100 50L63.44 63.44L50 100L36.56 63.44L0 50L36.56 36.56Z", SHC_POLY, SHK_PATH),
    (
        "Stern (5 Zacken)",
        "M50 0L61.8 38.2L100 38.2L69.1 61.8L80.9 100L50 76.39L19.1 100L30.9 61.8L0 38.2L38.2 38.2Z",
        SHC_POLY,
        SHK_PATH,
    ),
    (
        "Stern (6 Zacken)",
        "M50 0L66.66 25.02L100 25L83.31 50L100 75L66.66 74.98L50 100L33.34 74.98L0 75L16.69 50L0 25L33.34 25.02Z",
        SHC_POLY,
        SHK_PATH,
    ),
    ("Pfeil", "M0 25L50 25L50 0L100 50L50 100L50 75L0 75Z", SHC_ARROWS, SHK_PATH),
    ("Gekerbter Pfeil", "M0 25H60V0L100 50L60 100V75H0L15 50Z", SHC_ARROWS, SHK_PATH),
    ("Richtungspfeil", "M0 0H70L100 50L70 100H0Z", SHC_ARROWS, SHK_PATH),
    ("Chevron Pfeil", "M0 0H70L100 50L70 100H0L30 50Z", SHC_ARROWS, SHK_PATH),
    ("Sprechblase (eckig)", "M0 0H100V75H45L20 100L25 75H0Z", SHC_CALLOUTS, SHK_PATH),
    (
        "Sprechblase (abgerundet)",
        "M12 0H88A12 12 0 0 1 100 12V63A12 12 0 0 1 88 75H45L20 100L25 75H12A12 12 0 0 1 0 63V12A12 12 0 0 1 12 0Z",
        SHC_CALLOUTS,
        SHK_PATH,
    ),
    (
        "Sprechblase (oval)",
        "M19.22 71.52C-0.82 59 -5.92 36.44 7.37 19.1C20.65 1.76 48.09 -4.85 71.13 3.75C94.17 12.34 105.48 33.4 97.42 52.69C89.35 71.98 64.47 83.35 39.6 79.13L10 100Z",
        SHC_CALLOUTS,
        SHK_PATH,
    ),
    (
        "Sprechblase (Wolke)",
        concat!(
            "M42.43 11.72C45.32 -6.33 75.03 -2.38 72.15 15.67C83.53 4.86 101.31 20.43 89.93 31.23C103.6",
            "8 31.52 103.2 50.32 89.45 50.02C100.13 60.54 82.83 75.14 72.15 64.63C75.27 80.46 49.21 84.",
            "74 46.09 68.91C41.23 83.16 17.76 76.52 22.62 62.26C11.91 70.56 -1.73 55.91 8.98 47.62C-3.7",
            "1 46.95 -2.6 29.61 10.09 30.28C0.67 21.96 14.35 9.09 23.77 17.4C19.61 6.06 38.28 0.38 42.4",
            "3 11.72ZM34.35 82.49C34.35 84.88 31.8 86.82 28.65 86.82C25.51 86.82 22.95 84.88 22.95 82.4",
            "9C22.95 80.1 25.51 78.16 28.65 78.16C31.8 78.16 34.35 80.1 34.35 82.49ZM22.15 91.15C22.15 ",
            "92.85 20.34 94.23 18.1 94.23C15.86 94.23 14.04 92.85 14.04 91.15C14.04 89.45 15.86 88.07 1",
            "8.1 88.07C20.34 88.07 22.15 89.45 22.15 91.15ZM12.44 97.88C12.44 99.05 11.19 100 9.65 100C",
            "8.11 100 6.86 99.05 6.86 97.88C6.86 96.71 8.11 95.77 9.65 95.77C11.19 95.77 12.44 96.71 12",
            ".44 97.88Z"
        ),
        SHC_CALLOUTS,
        SHK_PATH,
    ),
    ("Häkchen", "M0 55L15 40L37 62L85 10L100 25L37 90Z", SHC_SYMBOLS, SHK_PATH),
    ("Multiplizieren", "M0 20L20 0L50 30L80 0L100 20L70 50L100 80L80 100L50 70L20 100L0 80L30 50Z", SHC_SYMBOLS, SHK_PATH),
    (
        "Zahnrad",
        concat!(
            "M41.4 0.74L58.6 0.74L58.51 11.94L70.9 17.07L78.75 9.09L90.91 21.25L82.93 29.1L88.06 41.49L",
            "99.26 41.4L99.26 58.6L88.06 58.51L82.93 70.9L90.91 78.75L78.75 90.91L70.9 82.93L58.51 88.0",
            "6L58.6 99.26L41.4 99.26L41.49 88.06L29.1 82.93L21.25 90.91L9.09 78.75L17.07 70.9L11.94 58.",
            "51L0.74 58.6L0.74 41.4L11.94 41.49L17.07 29.1L9.09 21.25L21.25 9.09L29.1 17.07L41.49 11.94",
            "ZM66 50A16 16 0 1 0 34 50A16 16 0 1 0 66 50Z"
        ),
        SHC_SYMBOLS,
        SHK_PATH,
    ),
    (
        "Herz",
        "M50 100C20 75.56 0 53.33 0 28.89C0 11.11 12 0 27 0C38 0 47 7.78 50 20C53 7.78 62 0 73 0C88 0 100 11.11 100 28.89C100 53.33 80 75.56 50 100Z",
        SHC_SYMBOLS,
        SHK_PATH,
    ),
    (
        "Blitz",
        "M39.22 0L59.54 28.15L51.16 31.47L76.75 55.59L68.37 59.62L100 100L46.35 69.05L56.58 64.76L23.25 44.93L35.19 38.81L0 18.01Z",
        SHC_SYMBOLS,
        SHK_PATH,
    ),
    (
        "Smartphone",
        "M12 0H88Q100 0 100 12V88Q100 100 88 100H12Q0 100 0 88V12Q0 0 12 0Z M8 14V84H92V14Z M38 6V9H62V6Z M56 92A6 4 0 1 0 44 92A6 4 0 1 0 56 92Z",
        SHC_SYMBOLS,
        SHK_PATH,
    ),
];
