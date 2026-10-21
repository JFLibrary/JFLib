// fonttest -- command-line checks of the runtime font loader (text/ttf.rs, cff.rs, colr.rs,
// fontdb.rs).  Every mode ends the process (exit code 0 ok, 1 error); the same command line
// and output as the assembly version, so tools/font_check.py and the scripts run unchanged.
//                                                                  [apps/fonttest/src/main.s]
//
//   --dump FILE [FACE]                 names, metrics, flags, glyph count
//   --outline FILE FACE CP             contours of one glyph (u CP, or gGID for a glyph id)
//   --outlines FILE FACE LIST OUT      batch: LIST lines "u CP" / "g GID" -> OUT records
//                                      "G key gid adv n xmin ymin xmax ymax" + "P tag x y"
//                                      (coordinates x 1024), "E" per contour end
//   --kern FILE FACE LIST OUT          LIST lines "CP1 CP2" -> OUT "cp1 cp2 g1 g2 kern ktable"
//   --render FILE FACE SIZE TEXT OUT.png [--style N] [--fg RRGGBB] [--bg RRGGBB] [--decor N]
//                                      [--subpixel none|rgb|bgr|vrgb|vbgr]
//                                      FILE "@N" = embedded font N; style = FS_* flags;
//                                      --subpixel: an opaque (XRGB) target and subpixel
//                                      smoothing in that order (text/lcd.rs)
//   --cmpsgf FILE FACE SGFID           glyph masks of FILE vs the embedded font SGFID at many
//                                      sizes / phases: must be identical (exit 1 otherwise);
//                                      code points FILE lacks (ttf2bin --compose / --donor)
//                                      are counted, not compared
//   --fuzz FILE                        load every face of a (possibly broken) file and walk
//                                      everything; must not crash
//   --bench FILE FACE                  outline extraction and rasterization speed
//   --selftest                         known-answer checks on the synthetic fixtures in
//                                      apps/fonttest/tests/fixtures (tests/selftest.uis)
//   --scan [OUT] [--cache F] [--rescan] fontdb scan: families and faces
//   --find FAMILY WEIGHT ITALIC        fontdb find
//   --conf FAMILY [LANG]               fontconf: the system's rules (files read), the
//                                      candidates and the family FontDb::resolve finds
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Dc, Surface, FMT_A8, FMT_PARGB, FMT_XRGB};
use rt::fx::cvt;
use text::fontconf::{is_generic, FontConf};
use text::fontdb::{FontDb, FDB_RESCAN};
use text::{GcRef, Text, FM_FLAGS, FMF_ITALIC, FMF_SYN, FN_FAMILY, FN_FULL, FN_STYLE, FN_TYPO_FAMILY,
           FN_TYPO_STYLE, FONT_ICON, FONT_REG, FS_SHARP, FS_SYN_BOLD, FS_SYN_ITALIC, SUBPIXEL_BGR, SUBPIXEL_NONE,
           SUBPIXEL_RGB, SUBPIXEL_VBGR, SUBPIXEL_VRGB};
use ui::Style;

ui::main!(fonttest());

const NSIZES: usize = 20;
static CMP_SIZES: [i32; NSIZES] = [
    8 * 256, 9 * 256, 10 * 256, 11 * 256, 12 * 256, 13 * 256, 14 * 256, 15 * 256, 16 * 256, 18 * 256,
    20 * 256, 22 * 256, 24 * 256, 26 * 256, 28 * 256, 32 * 256, 40 * 256, 48 * 256, 13 * 256 + 85,
    21 * 256 + 128,
];
static NAME_IDS: [u32; 5] = [FN_FAMILY, FN_STYLE, FN_FULL, FN_TYPO_FAMILY, FN_TYPO_STYLE];
static SAMPLE: &str = "Hamburgefonstiv AVATAR Äöü ß € fi \u{2192} \u{1F600} \u{4E2D}";

struct Ft {
    t: Text,
    db: FontDb,
    a: Vec<String>,
    ob: Vec<u8>,
    key: String,
    st_fail: u32,
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

// ------------------------------------------------------------------ number parsing (str.s)

// parse_int(s) -> (value, bytes used): spaces, sign, decimal digits (wrapping)
fn parse_int(s: &[u8]) -> (i32, usize) {
    let mut i = 0;
    while s.get(i) == Some(&b' ') {
        i += 1;
    }
    let mut neg = false;
    if s.get(i) == Some(&b'-') {
        neg = true;
        i += 1;
    } else if s.get(i) == Some(&b'+') {
        i += 1;
    }
    let mut v = 0i32;
    while let Some(&c) = s.get(i) {
        let d = c.wrapping_sub(b'0');
        if d > 9 {
            break;
        }
        v = v.wrapping_mul(10).wrapping_add(d as i32);
        i += 1;
    }
    (if neg { v.wrapping_neg() } else { v }, i)
}

// parse_hex(s) -> (value, bytes used): optional "0x" or "#"
fn parse_hex(s: &[u8]) -> (u32, usize) {
    let mut i = 0;
    if s.first() == Some(&b'#') {
        i = 1;
    } else if s.first() == Some(&b'0') && s.get(1).map(|c| c | 32) == Some(b'x') {
        i = 2;
    }
    let mut v = 0u32;
    while let Some(&c) = s.get(i) {
        let d = if c.is_ascii_digit() {
            c - b'0'
        } else if ((c | 32).wrapping_sub(b'a')) <= 5 {
            (c | 32) - b'a' + 10
        } else {
            break;
        };
        v = (v << 4) | d as u32;
        i += 1;
    }
    (v, i)
}

// num_parse(s) -> (value, bytes used): decimal, 0x hex, U+hex
fn num_parse(s: &[u8]) -> (i32, usize) {
    if s.len() >= 2 && s[0] == b'U' && s[1] == b'+' {
        let (v, n) = parse_hex(&s[2..]);
        return (v as i32, n + 2);
    }
    if s.len() >= 2 && s[0] == b'0' && (s[1] | 32) == b'x' {
        let (v, n) = parse_hex(&s[2..]);
        return (v as i32, n + 2);
    }
    parse_int(s)
}

// the lines of a file (split at LF, cut at the first NUL)
fn read_lines(path: &str) -> Option<Vec<Vec<u8>>> {
    let mut d = plat::read_file(path)?;
    if let Some(z) = d.iter().position(|&c| c == 0) {
        d.truncate(z);
    }
    if d.is_empty() {
        return Some(Vec::new());
    }
    let mut v: Vec<Vec<u8>> = d.split(|&c| c == b'\n').map(|l| l.to_vec()).collect();
    if d.last() == Some(&b'\n') {
        v.pop();
    }
    Some(v)
}

fn fonttest() -> i32 {
    let st = &style::STYLE;
    let t = Text::new(st.fonts(), st.font_names());
    let mut ft = Ft { t, db: FontDb::new(), a: plat::args(), ob: Vec::new(), key: String::new(), st_fail: 0 };
    let modes = ["--selftest", "--dump", "--outlines", "--outline", "--kern", "--render", "--cmpsgf", "--fuzz",
                 "--bench", "--scan", "--find", "--conf"];
    for m in modes {
        let i = ft.arg_index(m);
        if i < 0 {
            continue;
        }
        let i = i as usize;
        return match m {
            "--selftest" => ft.cmd_selftest(),
            "--dump" => ft.cmd_dump(i),
            "--outlines" => ft.cmd_outlines(i),
            "--outline" => ft.cmd_outline(i),
            "--kern" => ft.cmd_kern(i),
            "--render" => ft.cmd_render(i),
            "--cmpsgf" => ft.cmd_cmpsgf(i),
            "--fuzz" => ft.cmd_fuzz(i),
            "--bench" => ft.cmd_bench(i),
            "--scan" => ft.cmd_scan(i),
            "--conf" => ft.cmd_conf(i),
            _ => ft.cmd_find(i),
        };
    }
    plat::write_stderr(b"usage: fonttest --shot x.png (--dump F [FACE] | --outline F FACE CP | --outlines F FACE LIST OUT | --kern F FACE LIST OUT | --render F FACE SIZE TEXT OUT | --cmpsgf F FACE SGF | --fuzz F | --bench F FACE | --scan [OUT] | --find FAMILY W I | --conf FAMILY [LANG])\n");
    1
}

impl Ft {
    // ------------------------------------------------------------------ helpers

    // the index of the argument equal to name, or -1
    fn arg_index(&self, name: &str) -> i32 {
        self.a.iter().skip(1).position(|s| s == name).map_or(-1, |i| i as i32 + 1)
    }

    // argument i or ""
    fn arg_at(&self, i: usize) -> String {
        self.a.get(i).cloned().unwrap_or_default()
    }

    // integer argument i (decimal or 0x hex), def when absent / empty
    fn arg_num(&self, i: usize, def: i32) -> i32 {
        match self.a.get(i) {
            Some(s) if !s.is_empty() => num_parse(s.as_bytes()).0,
            _ => def,
        }
    }

    fn arg_value(&self, name: &str) -> Option<String> {
        let i = self.arg_index(name);
        if i < 0 { None } else { self.a.get(i as usize + 1).cloned() }
    }

    fn arg_int(&self, name: &str, def: i32) -> i32 {
        match self.arg_value(name) {
            Some(v) => parse_int(v.as_bytes()).0,
            None => def,
        }
    }

    // font id / ERR_* for path ("@N" = embedded font N)
    fn load_font(&mut self, path: &str, face: i32) -> i32 {
        if let Some(n) = path.strip_prefix('@') {
            return parse_int(n.as_bytes()).0;
        }
        let id = self.t.load_file(path, face as u32);
        if id < 0 {
            plat::write_stderr(format!("error: cannot load {path} ({id})\n").as_bytes());
        }
        id
    }

    fn ob(&mut self, s: &str) {
        self.ob.extend_from_slice(s.as_bytes());
    }

    fn ob_bytes(&mut self, s: &[u8]) {
        self.ob.extend_from_slice(s);
    }

    // write the buffer to path ("" = stdout) and empty it -> 0 / 1
    fn ob_flush(&mut self, path: &str) -> i32 {
        let r = if path.is_empty() {
            plat::write_stdout(&self.ob);
            0
        } else if plat::write_file(path, &self.ob) {
            0
        } else {
            1
        };
        self.ob.clear();
        r
    }

    fn name(&self, font: u32, which: u32) -> String {
        String::from_utf8_lossy(self.t.name(font, which)).into_owned()
    }

    // ------------------------------------------------------------------ --dump

    fn cmd_dump(&mut self, ai: usize) -> i32 {
        let face = self.arg_num(ai + 2, 0);
        let path = self.arg_at(ai + 1);
        let id = self.load_font(&path, face);
        if id < 0 {
            return 1;
        }
        let f = id as u32;
        self.ob(&format!("font {path} face {face} -> id {id}\n"));
        let s = format!("family: {}\nstyle: {}\n", self.name(f, FN_FAMILY), self.name(f, FN_STYLE));
        self.ob(&s);
        let s = format!("full: {}\n", self.name(f, FN_FULL));
        self.ob(&s);
        let s = format!("typo family: {}\ntypo style: {}\n", self.name(f, FN_TYPO_FAMILY), self.name(f, FN_TYPO_STYLE));
        self.ob(&s);
        let m = self.t.metrics(f).unwrap_or_default();
        self.ob(&format!("upem {} glyphs {} weight {} width {} flags 0x{:x}\n", m.upem, m.nglyphs, m.weight, m.width,
                         m.flags as u32));
        self.ob(&format!("hhea asc {} desc {} gap {} advmax {}\n", m.asc, m.desc, m.gap, m.advmax));
        self.ob(&format!("typo asc {} desc {} gap {}  win asc {} desc {}\n", m.typo_asc, m.typo_desc, m.typo_gap,
                         m.win_asc, m.win_desc));
        self.ob(&format!("xheight {} capheight {} underline {}/{} strikeout {}/{}\n", m.xh, m.caph, m.ul_pos,
                         m.ul_thick, m.st_pos, m.st_thick));
        let g = self.t.glyph(f, 'A' as u32);
        let adv = self.t.adv(f, g);
        self.ob(&format!("'A' -> glyph {g} adv {adv}\n"));
        self.ob_flush("");
        0
    }

    // ------------------------------------------------------------------ outlines

    // the record of one glyph  [put_outline]
    fn put_outline(&mut self, font: u32, gid: u32) {
        let b = self.t.glyph_box(font, gid);
        let bx = b.unwrap_or([0; 4]);
        let adv = self.t.adv(font, gid);
        let n = if b.is_some() { self.t.ol.ends.len() } else { 0 };
        let s = format!("G {} {} {} {} {} {} {} {}\n", self.key, gid as i32, adv, n, bx[0], bx[1], bx[2], bx[3]);
        self.ob(&s);
        if b.is_none() {
            return;
        }
        // contours from the outline buffer (glyph_box prepared it)
        let mut p = 0usize;
        let mut s = String::new();
        for &e in &self.t.ol.ends {
            while p <= e as usize {
                let (x, y) = self.t.ol.pts[p];
                let tag = self.t.ol.tags[p];
                s.push_str(&format!("P {} {} {}\n", tag, cvt(x * 1024.0), cvt(y * 1024.0)));
                p += 1;
            }
            s.push_str("E\n");
        }
        self.ob(&s);
    }

    // glyph for "gGID" / "u CP" / "CP" (the key text is kept)  [keyed_glyph]
    fn keyed_glyph(&mut self, font: u32, s: &[u8]) -> u32 {
        let mut k = &s[..s.len().min(63)];
        if k.last() == Some(&b'\r') {
            k = &k[..k.len() - 1];
        }
        self.key = String::from_utf8_lossy(k).into_owned();
        if s.first() == Some(&b'g') {
            return parse_int(&s[1..]).0 as u32;
        }
        let mut i = 0;
        if s.first() == Some(&b'u') {
            i = 1;
            while s.get(i) == Some(&b' ') {
                i += 1;
            }
        }
        let cp = num_parse(&s[i..]).0;
        self.t.glyph(font, cp as u32)
    }

    fn cmd_outline(&mut self, ai: usize) -> i32 {
        let face = self.arg_num(ai + 2, 0);
        let path = self.arg_at(ai + 1);
        let id = self.load_font(&path, face);
        if id < 0 {
            return 1;
        }
        let k = self.arg_at(ai + 3);
        let g = self.keyed_glyph(id as u32, k.as_bytes());
        self.put_outline(id as u32, g);
        self.ob_flush("");
        0
    }

    fn cmd_outlines(&mut self, ai: usize) -> i32 {
        let face = self.arg_num(ai + 2, 0);
        let path = self.arg_at(ai + 1);
        let id = self.load_font(&path, face);
        if id < 0 {
            return 1;
        }
        let Some(lines) = read_lines(&self.arg_at(ai + 3)) else { return 1 };
        for l in lines {
            if l.is_empty() || l[0] == 13 {
                continue;
            }
            let g = self.keyed_glyph(id as u32, &l);
            self.put_outline(id as u32, g);
        }
        let o = self.arg_at(ai + 4);
        self.ob_flush(&o)
    }

    // LIST lines "CP1 CP2"
    fn cmd_kern(&mut self, ai: usize) -> i32 {
        let face = self.arg_num(ai + 2, 0);
        let path = self.arg_at(ai + 1);
        let id = self.load_font(&path, face);
        if id < 0 {
            return 1;
        }
        let f = id as u32;
        let Some(lines) = read_lines(&self.arg_at(ai + 3)) else { return 1 };
        for l in lines {
            if l.is_empty() || l[0] < b'0' {
                continue;
            }
            let (cp1, n) = num_parse(&l);
            let mut i = n;
            while l.get(i) == Some(&b' ') {
                i += 1;
            }
            let (cp2, _) = num_parse(&l[i..]);
            let g1 = self.t.glyph(f, cp1 as u32);
            let g2 = self.t.glyph(f, cp2 as u32);
            let k = self.t.kern(f, g1, g2);
            let kt = self.t.kern_table(f, g1, g2);
            self.ob(&format!("{cp1} {cp2} {} {} {k} {kt}\n", g1 as i32, g2 as i32));
        }
        let o = self.arg_at(ai + 4);
        self.ob_flush(&o)
    }

    // ------------------------------------------------------------------ --render

    fn cmd_render(&mut self, ai: usize) -> i32 {
        let face = self.arg_num(ai + 2, 0);
        let path = self.arg_at(ai + 1);
        let mut id = self.load_font(&path, face);
        if id < 0 {
            return 1;
        }
        let st = self.arg_int("--style", 0);
        if st != 0 {
            id = self.t.style(id as u32, st as u32);
            if id < 0 {
                return 1;
            }
        }
        let mut fg = 0xFF00_0000u32;
        let mut bg = 0xFFFF_FFFFu32;
        if let Some(v) = self.arg_value("--fg") {
            fg = parse_hex(v.as_bytes()).0 | 0xFF00_0000;
        }
        if let Some(v) = self.arg_value("--bg") {
            bg = parse_hex(v.as_bytes()).0 | 0xFF00_0000;
        }
        let size = self.arg_num(ai + 3, 16) << 8;
        self.t.set_font(id as u32, size);
        let txt = self.arg_at(ai + 4);
        let asc = self.t.ascent();
        let desc = self.t.descent();
        let w = self.t.width(&txt) + 16;
        let h = asc + desc + 16;
        if w <= 0 || h <= 0 {
            return 1;
        }
        let sub = self.arg_value("--subpixel");
        if let Some(v) = &sub {
            let order = match v.as_str() {
                "rgb" => SUBPIXEL_RGB,
                "bgr" => SUBPIXEL_BGR,
                "vrgb" => SUBPIXEL_VRGB,
                "vbgr" => SUBPIXEL_VBGR,
                _ => SUBPIXEL_NONE,
            };
            self.t.set_subpixel(order);
        }
        let mut sf = Surface::new(w, h, if sub.is_some() { FMT_XRGB } else { FMT_PARGB });
        let mut dc = Dc::new(w, h);
        dc.begin(&sf);
        dc.clear(&mut sf, bg);
        dc.set_color(fg);
        let width = self.t.draw(&mut dc, &mut sf, 8, asc + 8, txt.as_bytes());
        let decor = self.arg_int("--decor", 0);
        if decor != 0 {
            self.t.decor(&dc, &mut sf, 8, asc + 8, width, decor as u32);
        }
        dc.end();
        let png = img_png::encode(&sf.px, w as usize, h as usize, sf.stride as usize, false);
        if plat::write_file(&self.arg_at(ai + 5), &png) { 0 } else { 1 }
    }

    // ------------------------------------------------------------------ --cmpsgf

    fn cmd_cmpsgf(&mut self, ai: usize) -> i32 {
        let face = self.arg_num(ai + 2, 0);
        let path = self.arg_at(ai + 1);
        let id = self.load_font(&path, face);
        if id < 0 {
            return 1;
        }
        let rt = id as u32;
        let sgf = self.arg_num(ai + 3, 0) as u32;
        let (mut bad, mut compared, mut advbad, mut extra) = (0u32, 0u32, 0u32, 0u32);
        for cp in 0x20u32..0x10000 {
            if !self.t.has(sgf, cp) {
                continue;
            }
            // built by ttf2bin --compose / --donor: FILE has nothing to compare it with
            if !self.t.has(rt, cp) {
                extra += 1;
                continue;
            }
            let gs = self.t.glyph(sgf, cp);
            let gr = self.t.glyph(rt, cp);
            let a1 = self.t.adv(sgf, gs);
            let a2 = self.t.adv(rt, gr);
            if a1 != a2 {
                advbad += 1;
                out(&format!("adv U+{cp:04X}: sgf {a1} ttf {a2}\n"));
            }
            for &size in CMP_SIZES.iter() {
                for phase in 0..4 {
                    // copy the SGF1 entry (the next gc_get may flush the cache)
                    let e1 = match self.t.gc_get(sgf, gs, size, phase) {
                        GcRef::Entry(e) => e.clone(),
                        _ => continue,
                    };
                    compared += 1;
                    let same = match self.t.gc_get(rt, gr, size, phase) {
                        GcRef::Entry(e) => {
                            e.left as i16 == e1.left as i16
                                && e.top as i16 == e1.top as i16
                                && e.w as u16 == e1.w as u16
                                && e.h as u16 == e1.h as u16
                                && e.px == e1.px
                        }
                        _ => false,
                    };
                    if !same {
                        bad += 1;
                        if bad <= 20 {
                            out(&format!("mask U+{cp:04X} size_fx {size} phase {phase} differs\n"));
                        }
                    }
                }
            }
        }
        out(&format!(
            "cmpsgf: {compared} masks compared, {bad} differ, {advbad} advances differ, {extra} code points only in the SGF1 font\n"
        ));
        ((bad | advbad) != 0) as i32
    }

    // ------------------------------------------------------------------ --fuzz

    fn cmd_fuzz(&mut self, ai: usize) -> i32 {
        if let Some(data) = plat::read_file(&self.arg_at(ai + 1)) {
            let n = Text::face_count(&data);
            for face in 0..n.clamp(0, 4) as u32 {
                let id = self.t.load_mem(&data, face);
                if id < 0 {
                    continue;
                }
                let f = id as u32;
                let m = self.t.metrics(f).unwrap_or_default();
                for w in NAME_IDS {
                    let _ = self.t.name(f, w).len();
                }
                // every glyph: box + path
                let ng = (m.nglyphs as u32).min(20000);
                self.t.gp = text::Gp { x: 0.0, y: 100.0, sx: 0.01, sy: 0.01, dx: 0, dy: 0 };
                let mut layers = Vec::new();
                for g in 0..ng {
                    self.t.glyph_box(f, g);
                    self.t.path.reset();
                    self.t.glyph_path(f, g);
                    self.t.adv(f, g);
                    self.t.layers(f, g, 8, &mut layers);
                    self.t.kern(f, g, g.wrapping_mul(8).wrapping_add(3) % ng);
                }
                let mut cp = 0u32;
                while cp < 0x11_0000 {
                    self.t.glyph(f, cp);
                    self.t.has(f, cp);
                    cp += 1;
                    if cp >= 0x3000 {
                        cp += 96;
                    }
                }
                // render text at a few sizes (glyph cache, colour glyphs, direct fills)
                let mut sf = Surface::new(400, 300, FMT_PARGB);
                let mut dc = Dc::new(400, 300);
                dc.begin(&sf);
                dc.set_color(0xFF10_2030);
                self.t.set_font(f, 14 * 256);
                self.t.draw(&mut dc, &mut sf, 4, 20, SAMPLE.as_bytes());
                self.t.set_font(f, 300 * 256);
                self.t.draw(&mut dc, &mut sf, 4, 280, SAMPLE.as_bytes());
                let s = self.t.style(f, FS_SYN_BOLD | FS_SYN_ITALIC | FS_SHARP);
                if s >= 0 {
                    self.t.set_font(s as u32, 18 * 256);
                    self.t.draw(&mut dc, &mut sf, 4, 60, SAMPLE.as_bytes());
                }
                dc.end();
                self.t.set_font(FONT_REG, 14 * 256);
                self.t.unload(f);
            }
        }
        out("fuzz ok\n");
        0
    }

    // ------------------------------------------------------------------ --bench

    fn cmd_bench(&mut self, ai: usize) -> i32 {
        let face = self.arg_num(ai + 2, 0);
        let t0 = plat::time_us();
        let path = self.arg_at(ai + 1);
        let id = self.load_font(&path, face);
        if id < 0 {
            return 1;
        }
        let f = id as u32;
        out(&format!("load: {} us\n", (plat::time_us() - t0) as i32));
        let ng = self.t.metrics(f).unwrap_or_default().nglyphs as u32;
        // outline extraction: every glyph, 3 rounds
        let t0 = plat::time_us();
        let mut non = 0;
        for _ in 0..3 {
            for g in 0..ng {
                non += self.t.glyph_box(f, g).is_some() as i32;
            }
        }
        out(&format!("outlines: {} glyphs x 3 ({} non-empty) in {} us\n", ng, non, (plat::time_us() - t0) as i32));
        // rasterization into the glyph cache at 16 px
        self.t.gc_flush();
        let t0 = plat::time_us();
        let n = ng.min(2000);
        for g in 0..n {
            let _ = self.t.gc_get(f, g, 16 * 256, 0);
        }
        out(&format!("raster 16px: {} glyphs in {} us\n", n, (plat::time_us() - t0) as i32));
        0
    }

    // ------------------------------------------------------------------ fontdb

    fn cmd_scan(&mut self, ai: usize) -> i32 {
        let t0 = plat::time_us();
        let cache = self.arg_value("--cache");
        if let Some(c) = &cache {
            self.db.load_cache(c);
        }
        let flags = if self.arg_index("--rescan") >= 0 { FDB_RESCAN } else { 0 };
        let n = self.db.scan(flags);
        if let Some(c) = &cache {
            self.db.save_cache(c);
        }
        let us = (plat::time_us() - t0) as i32;
        let nf = self.db.family_count();
        self.ob(&format!("# {n} faces, {nf} families, {us} us\n"));
        let mut s = String::new();
        for i in 0..nf {
            s.push_str(&format!("F {}\n", self.db.family_name(i)));
            for f in self.db.family_faces(i) {
                s.push_str(&format!("  {} {} 0x{:x} {} | {} | {}", f.weight, f.face, f.flags, f.style, f.full, f.tfamily));
                s.push_str(&format!(" | {}\n", f.path));
            }
        }
        self.ob(&s);
        let mut o = self.arg_at(ai + 1);
        if o.starts_with('-') {
            o = String::new();
        }
        self.ob_flush(&o)
    }

    fn cmd_conf(&mut self, ai: usize) -> i32 {
        let fam = self.arg_at(ai + 1);
        let lang = self.arg_at(ai + 2);
        let c = FontConf::system();
        out(&format!("# {} fontconfig file(s)\n", c.files()));
        for f in c.candidates(&fam, &lang) {
            out(&format!("  {}\n", f));
        }
        let lf = c.lang_families(&lang);
        if !lf.is_empty() {
            out(&format!("# language fonts: {}\n", lf.join(", ")));
        }
        self.db.scan(0);
        match self.db.resolve(&c, &fam, &lang, 400, false) {
            Some((i, _, f)) => {
                out(&format!("-> {} ({})\n", f, self.db.face(i).map_or("", |x| x.path.as_str())));
                0
            }
            None => {
                out("-> not found\n");
                1
            }
        }
    }

    fn cmd_find(&mut self, ai: usize) -> i32 {
        self.db.scan(0);
        let w = self.arg_num(ai + 2, 400);
        let it = self.arg_num(ai + 3, 0);
        let fam = self.arg_at(ai + 1);
        match self.db.find(&fam, w as u32, it != 0) {
            Some((i, syn)) => {
                let f = self.db.face(i).unwrap();
                out(&format!("{} face {} weight {} syn {} ({})\n", f.path, f.face, f.weight, syn, f.full));
                0
            }
            None => {
                out("not found\n");
                1
            }
        }
    }

    // ------------------------------------------------------------------ --selftest

    // the rules of fixtures/fontconf/fonts.conf (and its conf.d) and FontDb::resolve on the
    // fixture fonts (scanned by the caller)
    fn st_fontconf(&mut self) {
        let mut c = FontConf::new();
        let ok = c.load("apps/fonttest/tests/fixtures/fontconf/fonts.conf");
        self.st_check(ok && c.files() == 3, &format!("selftest: fontconf files {} (3)\n", c.files()));
        let l = c.candidates("sans-serif", "de");
        let want = ["jfgui Missing", "jfgui Mini", "jfgui CID", "sans-serif", "Accept Two", "Accept One", "jfgui Mac"];
        self.st_check(l == want, &format!("selftest: fontconf alias order {l:?}\n"));
        let l = c.candidates("SANS-SERIF", "ja-JP");
        self.st_check(l.get(3).map(String::as_str) == Some("JP Font") && l.len() == 8, &format!("selftest: fontconf lang prepend {l:?}\n"));
        let l = c.candidates("sans-serif", "zh-Hant-TW");
        self.st_check(l.first().map(String::as_str) == Some("TC Font"), &format!("selftest: fontconf zh-Hant -> zh-tw {l:?}\n"));
        let ja = c.lang_families("ja");
        self.st_check(ja == ["JP Font"] && c.lang_families("de").is_empty(), &format!("selftest: fontconf lang_families {ja:?}\n"));
        let l = c.candidates("monospace", "");
        self.st_check(l == ["Mono A", "Mono B", "monospace"], &format!("selftest: fontconf conf.d in name order {l:?}\n"));
        let l = c.candidates("Old Name", "");
        self.st_check(l == ["jfgui Mac"], &format!("selftest: fontconf assign {l:?}\n"));
        let l = c.candidates("Helvetica & Co", "");
        self.st_check(l == ["Helvetica & Co", "jfgui Mini CFF"], &format!("selftest: fontconf entities {l:?}\n"));
        let never = ["sans-serif", "monospace", "x"].iter().any(|f| c.candidates(f, "zh-cn").iter().any(|n| n == "Never"));
        self.st_check(!never && is_generic("Monospace") && !is_generic("Mono A"), "selftest: fontconf skips charset rules, generics\n");
        let mut found = alloc::vec::Vec::new();
        for f in ["sans-serif", "Helvetica & Co", "Unknown Family", "Old Name"] {
            found.push(self.db.resolve(&c, f, "de", 400, false).map(|r| r.2).unwrap_or_default());
        }
        self.st_check(found == ["jfgui Mini", "jfgui Mini CFF", "jfgui Mini", "jfgui Mac"], &format!("selftest: fontdb resolve {found:?}\n"));
        let (id, fam) = self.db.load_resolved(&mut self.t, &c, "sans-serif", "de", 700, false);
        self.st_check(id >= 0 && fam == "jfgui Mini", &format!("selftest: fontdb load_resolved {id} {fam}\n"));
        // a rule of text: lang rules without a family test, an unclosed element
        let mut c = FontConf::new();
        let n = c.parse("<fontconfig><match><test name=\"lang\"><string>ko</string></test><edit name=\"family\" mode=\"append_last\"><string>KR</string></edit></match><alias><family>serif</family><prefer><family>S</family>", "");
        let l = c.candidates("serif", "ko");
        self.st_check(n == 2 && l == ["S", "serif", "KR"], &format!("selftest: fontconf parse {n} {l:?}\n"));
    }

    // text s in font / size on white (A8: on 0) in colour c, drawn with a subpixel order into
    // a target of format fmt
    fn st_draw(&mut self, font: u32, size: i32, order: u8, fmt: u8, c: u32, s: &str) -> Surface {
        self.t.set_font(font, size);
        self.t.set_subpixel(order);
        let (w, h) = (220, 32);
        let mut sf = Surface::new(w, h, fmt);
        let mut dc = Dc::new(w, h);
        dc.begin(&sf);
        dc.clear(&mut sf, if fmt == FMT_A8 { 0 } else { 0xFFFF_FFFF });
        dc.set_color(c);
        self.t.draw(&mut dc, &mut sf, 5, 22, s.as_bytes());
        dc.end();
        self.t.set_subpixel(SUBPIXEL_NONE);
        sf
    }

    // lcd.rs: greyscale stays neutral; RGB / BGR (VRGB / VBGR) are each other's mirror; only
    // opaque targets, never colour glyphs or the icon font; the cache keeps the modes apart;
    // the soft hyphen is invisible
    fn st_subpixel(&mut self) {
        let exact = self.t.lcd_tables_exact();
        self.st_check(exact, "selftest: subpixel blend tables lose a value\n");
        let px = |sf: &Surface| -> Vec<[u8; 3]> {
            let mut v = Vec::new();
            for y in 0..sf.h as usize {
                for x in 0..sf.w as usize {
                    let o = y * sf.stride as usize + x * 4;
                    v.push([sf.px[o + 2], sf.px[o + 1], sf.px[o]]);
                }
            }
            v
        };
        let text = "Hamburgefonstiv 0123 Wiley";
        let sz = 13 * 256;
        let black = 0xFF00_0000;
        let grey = px(&self.st_draw(FONT_REG, sz, SUBPIXEL_NONE, FMT_XRGB, black, text));
        let rgb = px(&self.st_draw(FONT_REG, sz, SUBPIXEL_RGB, FMT_XRGB, black, text));
        let bgr = px(&self.st_draw(FONT_REG, sz, SUBPIXEL_BGR, FMT_XRGB, black, text));
        let vrgb = px(&self.st_draw(FONT_REG, sz, SUBPIXEL_VRGB, FMT_XRGB, black, text));
        let vbgr = px(&self.st_draw(FONT_REG, sz, SUBPIXEL_VBGR, FMT_XRGB, black, text));
        let neutral = |v: &[[u8; 3]]| v.iter().all(|p| p[0] == p[1] && p[1] == p[2]);
        let inked = |v: &[[u8; 3]]| v.iter().filter(|p| p[0] < 128 || p[1] < 128 || p[2] < 128).count();
        let swapped = |a: &[[u8; 3]], b: &[[u8; 3]]| a.iter().zip(b).all(|(p, q)| p[0] == q[2] && p[1] == q[1] && p[2] == q[0]);
        self.st_check(neutral(&grey) && inked(&grey) > 100, "selftest: greyscale text is not neutral\n");
        self.st_check(!neutral(&rgb) && !neutral(&vrgb), "selftest: subpixel text has no colour\n");
        self.st_check(swapped(&rgb, &bgr) && rgb != bgr, "selftest: BGR is not RGB mirrored\n");
        self.st_check(swapped(&vrgb, &vbgr) && vrgb != rgb, "selftest: VBGR is not VRGB mirrored\n");
        // the ink stays about as heavy as greyscale text (gamma, enhanced contrast): black on
        // white and white on dark grey within 10 %
        let ink = |v: &[[u8; 3]]| v.iter().map(|p| 765 - (p[0] as u32 + p[1] as u32 + p[2] as u32)).sum::<u32>();
        let (gi, ri) = (ink(&grey), ink(&rgb));
        self.st_check(ri * 10 > gi * 9 && ri * 10 < gi * 11, &format!("selftest: subpixel ink {ri} vs greyscale {gi}\n"));
        let light = |me: &mut Self, order: u8| -> u32 {
            me.t.set_font(FONT_REG, sz);
            me.t.set_subpixel(order);
            let mut sf = Surface::new(220, 32, FMT_XRGB);
            let mut dc = Dc::new(220, 32);
            dc.begin(&sf);
            dc.clear(&mut sf, 0xFF20_2020);
            dc.set_color(0xFFFF_FFFF);
            me.t.draw(&mut dc, &mut sf, 5, 22, text.as_bytes());
            dc.end();
            me.t.set_subpixel(SUBPIXEL_NONE);
            sf.px.chunks_exact(4).map(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 - 3 * 0x20).sum()
        };
        let (gl, rl) = (light(self, SUBPIXEL_NONE), light(self, SUBPIXEL_RGB));
        self.st_check(rl * 10 > gl * 9 && rl * 10 < gl * 11, &format!("selftest: subpixel white ink {rl} vs greyscale {gl}\n"));
        // the greyscale masks are still there after the subpixel ones were made
        let again = px(&self.st_draw(FONT_REG, sz, SUBPIXEL_NONE, FMT_XRGB, black, text));
        self.st_check(again == grey, "selftest: greyscale text changed after subpixel text\n");
        // layers (PARGB) and masks (A8) stay greyscale
        for fmt in [FMT_PARGB, FMT_A8] {
            let a = self.st_draw(FONT_REG, sz, SUBPIXEL_NONE, fmt, black, text);
            let b = self.st_draw(FONT_REG, sz, SUBPIXEL_RGB, fmt, black, text);
            self.st_check(a.px == b.px, &format!("selftest: subpixel text on a format {fmt} target\n"));
        }
        // translucent text: lighter, still coloured at the edges
        let half = px(&self.st_draw(FONT_REG, sz, SUBPIXEL_RGB, FMT_XRGB, 0x8000_0000, text));
        self.st_check(ink(&half) < ri && !neutral(&half), "selftest: translucent subpixel text\n");
        // the icon font stays greyscale
        let icon = "\u{E713}\u{E721}";
        let a = self.st_draw(FONT_ICON, 16 * 256, SUBPIXEL_NONE, FMT_XRGB, black, icon);
        let b = self.st_draw(FONT_ICON, 16 * 256, SUBPIXEL_RGB, FMT_XRGB, black, icon);
        self.st_check(a.px == b.px, "selftest: the icon font got subpixel smoothing\n");
        // black and white masks (font smoothing off) stay so
        self.t.set_antialias(false);
        let a = self.st_draw(FONT_REG, sz, SUBPIXEL_NONE, FMT_XRGB, black, text);
        let b = self.st_draw(FONT_REG, sz, SUBPIXEL_RGB, FMT_XRGB, black, text);
        self.t.set_antialias(true);
        self.st_check(a.px == b.px, "selftest: subpixel smoothing with smoothing off\n");
        // a COLR glyph (mini.ttf "C") is greyscale, its plain neighbour "A" is not
        let id = self.t.load_file("apps/fonttest/tests/fixtures/mini.ttf", 0);
        if id >= 0 {
            let f = id as u32;
            let a = self.st_draw(f, 20 * 256, SUBPIXEL_NONE, FMT_XRGB, black, "C");
            let b = self.st_draw(f, 20 * 256, SUBPIXEL_RGB, FMT_XRGB, black, "C");
            self.st_check(a.px == b.px, "selftest: a COLR glyph got subpixel smoothing\n");
            let a = self.st_draw(f, 20 * 256, SUBPIXEL_NONE, FMT_XRGB, black, "A");
            let b = self.st_draw(f, 20 * 256, SUBPIXEL_RGB, FMT_XRGB, black, "A");
            self.st_check(a.px != b.px, "selftest: mini.ttf A without subpixel smoothing\n");
            self.t.unload(f);
        }
        // a soft hyphen is invisible and has no width
        self.t.set_font(FONT_REG, sz);
        let (w1, w2) = (self.t.measure("Silben"), self.t.measure("Sil\u{AD}ben"));
        self.st_check(w1 == w2, &format!("selftest: soft hyphen width {w1} / {w2}\n"));
        let a = self.st_draw(FONT_REG, sz, SUBPIXEL_NONE, FMT_XRGB, black, "Silben");
        let b = self.st_draw(FONT_REG, sz, SUBPIXEL_NONE, FMT_XRGB, black, "Sil\u{AD}ben");
        self.st_check(a.px == b.px, "selftest: a soft hyphen was drawn\n");
    }

    // fallback.rs: system fallback from jfgui_FONTDIRS, the app's chain, never-fallback marks
    fn st_fallback(&mut self) {
        plat::setenv("jfgui_FONTDIRS", Some("lib/doc_view/assets"));
        plat::setenv("jfgui_FALLBACK_FONTS", Some("DejaVu Serif"));
        self.t.set_font(FONT_REG, 16 << 8);
        // a character of DejaVu Serif that the UI font lacks
        let Some(cp) = [0x0416u32, 0x03C8, 0x2211, 0x2318, 0x263A].into_iter().find(|&c| !self.t.has(FONT_REG, c)) else {
            out("selftest: fallback skipped (the UI font has every test character)\n");
            return;
        };
        let s = alloc::format!("a{}b", char::from_u32(cp).unwrap_or('?'));
        let render = |t: &mut Text| {
            let mut sf = Surface::new(64, 32, FMT_A8);
            let mut dc = Dc::new(64, 32);
            dc.set_color(0xFF00_0000);
            t.draw(&mut dc, &mut sf, 2, 24, s.as_bytes());
            sf.px
        };
        self.t.set_system_fallback(false);
        let w0 = self.t.measure(&s);
        let tofu = render(&mut self.t);
        let none = self.t.fallback_for(cp).is_none();
        self.st_check(none, "selftest: fallback without a chain\n");
        self.t.set_system_fallback(true);
        let fb = self.t.fallback_for(cp);
        let has = fb.is_some_and(|f| self.t.has(f, cp));
        self.st_check(has, &alloc::format!("selftest: system fallback for U+{cp:04X} {fb:?}\n"));
        let again = self.t.fallback_for(cp);
        self.st_check(again == fb, "selftest: fallback cache\n");
        let marks = self.t.fallback_for(0x200B).is_none() && self.t.fallback_for(0x0A).is_none();
        self.st_check(marks, "selftest: zero width / controls never fall back\n");
        let w1 = self.t.measure(&s);
        let drawn = render(&mut self.t);
        self.st_check(drawn != tofu, "selftest: the fallback glyph is drawn, not the missing glyph\n");
        self.st_check(w1 > 0 && w0 > 0, &alloc::format!("selftest: fallback widths {w0} / {w1}\n"));
        // the app's own chain without system fonts
        self.t.set_system_fallback(false);
        if let Some(f) = fb {
            self.t.set_fallback_fonts(&[f]);
            let app = self.t.fallback_for(cp);
            self.st_check(app == Some(f), "selftest: the app's fallback chain\n");
            let same = render(&mut self.t) == drawn;
            self.st_check(same, "selftest: the app's chain draws the same glyph\n");
            self.t.set_fallback_fonts(&[]);
        }
        let none = self.t.fallback_for(cp).is_none();
        self.st_check(none, "selftest: no chain, no fallback\n");
        plat::setenv("jfgui_FALLBACK_FONTS", None);
    }

    // count a failed check and print msg
    fn st_check(&mut self, ok: bool, msg: &str) {
        if !ok {
            self.st_fail += 1;
            out(msg);
        }
    }

    // dump one fixture font as expected.txt lists it  [st_font]
    fn st_font(&mut self, path: &str, show: &str, face: u32, layer_glyph: u32, cps: &[u32], pairs: &[(u32, u32)]) {
        let id = self.t.load_file(path, face);
        if id < 0 {
            self.st_fail += 1;
            out(&format!("selftest: cannot load {path}\n"));
            return;
        }
        let f = id as u32;
        self.ob(&format!("font {show} {face}\n"));
        for w in NAME_IDS {
            self.ob(&format!("name {w} "));
            let n = self.t.name(f, w).to_vec();
            self.ob_bytes(&n);
            self.ob("\n");
        }
        let m = self.t.metrics(f).unwrap_or_default();
        self.ob(&format!("metrics {} {} {} {} {} {} ", m.upem, m.asc, m.desc, m.gap, m.xh, m.caph));
        self.ob(&format!("{} {} {} {} {} {:x} ", m.ul_pos, m.ul_thick, m.st_pos, m.st_thick, m.weight, m.flags as u32));
        self.ob(&format!("{}\n", m.nglyphs));
        for &cp in cps {
            let g = self.t.glyph(f, cp);
            self.ob(&format!("cmap {cp} {}\n", g as i32));
        }
        for g in 0..m.nglyphs.max(0) as u32 {
            self.key = format!("g{g}");
            self.put_outline(f, g);
        }
        for &(c1, c2) in pairs {
            let g1 = self.t.glyph(f, c1);
            let g2 = self.t.glyph(f, c2);
            let k = self.t.kern(f, g1, g2);
            let kt = self.t.kern_table(f, g1, g2);
            self.ob(&format!("kern {c1} {c2} {k} {kt}\n"));
        }
        let mut layers = Vec::new();
        self.t.layers(f, layer_glyph, 8, &mut layers);
        let mut s = format!("layers {} {}", layer_glyph, layers.len());
        for &(g, c) in &layers {
            s.push_str(&format!(" {g:x} {c:x}"));
        }
        s.push('\n');
        self.ob(&s);
        self.t.unload(f);
    }

    // load every truncation (step 7) of a fixture from memory and walk it  [st_fuzz]
    fn st_fuzz(&mut self, path: &str) {
        let Some(d) = plat::read_file(path) else { return };
        let mut n = 12;
        while n < d.len() {
            let id = self.t.load_mem(&d[..n], 0);
            if id >= 0 {
                for g in 0..16 {
                    self.t.glyph_box(id as u32, g);
                    self.t.kern(id as u32, g, 8);
                }
                self.t.unload(id as u32);
            }
            n += 7;
        }
    }

    fn cmd_selftest(&mut self) -> i32 {
        const FX: &str = "apps/fonttest/tests/fixtures/";
        let cps_tt = [0x41, 0x56, 0x4F, 0xC4, 0x53, 0x43, 0x20, 0x42, 0x1F600];
        let cps_mac = [0x41, 0x56, 0x4F, 0xC4, 0x48, 0x42, 0x1F600];
        let cps_cid = [0x41, 0x42, 0x43, 0x1F600, 0x20, 0x56, 0x4F];
        let cps_otf = [0x41, 0x56, 0x4F, 0x42, 0x46, 0x47, 0x4A, 0x51, 0x1F600, 0x43];
        let pairs = [(0x41, 0x56), (0x56, 0x41), (0x4F, 0x41), (0x41, 0x4F)];
        self.st_fail = 0;
        self.ob.clear();
        self.st_font(&format!("{FX}mini.ttf"), "mini.ttf", 0, 8, &cps_tt, &pairs);
        self.st_font(&format!("{FX}mini.ttc"), "mini.ttc", 1, 8, &cps_tt, &pairs);
        self.st_font(&format!("{FX}mini.otf"), "mini.otf", 0, 3, &cps_otf, &pairs);
        self.st_font(&format!("{FX}mac.ttf"), "mac.ttf", 0, 1, &cps_mac, &pairs);
        self.st_font(&format!("{FX}cid.otf"), "cid.otf", 0, 1, &cps_cid, &pairs);
        // ---- compare with expected.txt
        match plat::read_file(&format!("{FX}expected.txt")) {
            Some(e) => {
                if e != self.ob {
                    self.st_fail += 1;
                    out("selftest: output differs from expected.txt (actual output follows)\n");
                    self.ob_flush("");
                }
            }
            None => {
                self.st_fail += 1;
                out("selftest: expected.txt missing\n");
            }
        }
        self.ob.clear();
        // ---- collections
        match plat::read_file(&format!("{FX}mini.ttc")) {
            Some(d) => {
                let n = Text::face_count(&d);
                self.st_check(n == 2, &format!("selftest: mini.ttc face count {n}\n"));
            }
            None => self.st_fail += 1,
        }
        // ---- CBDT: bitmap entries 32 x 32 at 32 px, 16 x 16 at 16 px
        let id = self.t.load_file(&format!("{FX}cbdt.ttf"), 0);
        self.st_check(id >= 0, &format!("selftest: cbdt.ttf {id}\n"));
        if id >= 0 {
            let f = id as u32;
            let ok = self.t.gc_get_bitmap(f, 1, 32 * 256).is_some_and(|e| e.w == 32 && e.h == 32);
            self.st_check(ok, "selftest: CBDT 32 px entry wrong\n");
            let ok = self.t.gc_get_bitmap(f, 2, 16 * 256).is_some_and(|e| e.w == 16 && e.h == 16);
            self.st_check(ok, "selftest: CBDT 16 px entry wrong\n");
            self.t.unload(f);
        }
        // ---- synthetic bold: advance + upem / 30
        let id = self.t.load_file(&format!("{FX}mini.ttf"), 0);
        if id >= 0 {
            let f = id as u32;
            let s1 = self.t.style(f, FS_SYN_BOLD | FS_SYN_ITALIC);
            let s2 = self.t.style(f, FS_SYN_BOLD | FS_SYN_ITALIC);
            self.st_check(s1 == s2, &format!("selftest: font_style gave a new id {s2}\n"));
            let a = self.t.adv(s1 as u32, 1);
            self.st_check(a == 1033, &format!("selftest: bold advance {a}\n"));
            self.t.unload(f);
            let v = self.t.valid(s1 as u32);
            self.st_check(!v, "selftest: derived font survived font_unload\n");
        }
        // ---- fontdb on the fixtures folder
        plat::setenv("jfgui_FONTDIRS", Some("apps/fonttest/tests/fixtures"));
        let nf = self.db.scan(FDB_RESCAN);
        let nfam = self.db.family_count();
        self.st_check(nf == 6 && nfam == 5, &format!("selftest: fontdb faces {nf} (6) / families 5\n"));
        let ok = matches!(self.db.find("jfgui mini", 700, false), Some((i, 0)) if self.db.face(i).unwrap().weight == 700);
        self.st_check(ok, "selftest: fontdb_find bold\n");
        let ok = matches!(self.db.find("jfgui Mini", 400, true),
                          Some((i, s)) if self.db.face(i).unwrap().weight == 400 && s == FS_SYN_ITALIC);
        self.st_check(ok, "selftest: fontdb_find synthetic italic\n");
        let ok = matches!(self.db.find("jfgui Mini Typo", 400, false), Some((i, _)) if self.db.face(i).unwrap().face == 1);
        self.st_check(ok, "selftest: fontdb_find typographic family\n");
        let id = self.db.load(&mut self.t, "jfgui Mini", 700, true);
        let ok = id >= 0 && self.t.metric(id as u32, FM_FLAGS) & (FMF_ITALIC | FMF_SYN) == FMF_ITALIC | FMF_SYN;
        self.st_check(ok, &format!("selftest: fontdb_load bold italic -> {id}\n"));
        // cache round trip
        let cp = format!("{}/jfgui-fonttest-selftest.cache", plat::temp_dir().unwrap_or_default());
        let r = self.db.save_cache(&cp) | self.db.load_cache(&cp);
        let n = self.db.scan(0);
        self.st_check(r == 0 && n == 6, &format!("selftest: fontdb cache round trip {n}\n"));
        plat::delete(&cp);
        // ---- fontconf: generic families, substitutes, languages (fixtures/fontconf)
        self.st_fontconf();
        // ---- fallback: a character the UI font lacks comes from DejaVu Serif (lib/doc_view)
        self.st_fallback();
        // ---- subpixel smoothing (text/lcd.rs), the soft hyphen
        self.st_subpixel();
        // ---- truncated fixtures must not crash
        for f in ["mini.ttf", "mini.otf", "mini.ttc", "cbdt.ttf", "mac.ttf", "cid.otf"] {
            self.st_fuzz(&format!("{FX}{f}"));
        }
        // ---- result
        if self.st_fail == 0 {
            out("selftest: all checks passed\n");
            0
        } else {
            out(&format!("selftest: {} check(s) failed\n", self.st_fail));
            1
        }
    }
}
