// efftest -- command-line runner for the effect / adjustment kernels of the fx crate
// (contract: lib/fx/lib.rs, JFLIB/inc/fx.inc)                       [apps/efftest/src/main.s]
//
// Driven by tools/fx_check.py, fx_check_b.py and fx_check_c.py (the same protocol as the
// assembly version, so the tools run unchanged against this build):
//
//   efftest --shot - --list                  every registered effect and its properties
//   efftest --shot - --props NAME            the properties of one effect
//   efftest --shot - --fx NAME OPTIONS       run one effect
//   efftest --shot - --ops FILE              one run per line of FILE (OPTIONS incl. --fx;
//                                            empty lines and lines starting with '#' skipped)
//   efftest --shot - --selftest [all]        built-in checks (selftest.rs)
// OPTIONS
//   --in FILE             source image: ".raw" / ".argb" = RSR1 file {u32 "RSR1", u32 w,
//                         u32 h, u32 fmt, h rows of w * 4 bytes}, else any image format
//                         (straight ARGB)
//   --out FILE            result (".raw" / ".argb" raw as above with fmt 3, else by extension)
//   --p key=value         set a property (repeatable): int / seed decimal; dec / angle f64;
//                         bool 0/1/true/false/yes/no/on/off; choice key or index; pan "x,y";
//                         color hex AARRGGBB (6 digits or fewer: alpha 255); blob hex bytes
//   --threads N           1 = every tile on this thread in order (no pool); else the pool
//   --tile N              tile edge (default 256); FXF_BANDS: full-width bands of N rows;
//                         FXF_WHOLE: one render call over the whole image
//   --roi x0,y0,x1,y1     render only this rectangle (clamped; default the whole image)
//   --sel FILE            selection mask image of the same size (red channel = coverage):
//                         FxArgs::sel, afterwards out = lerp(src, result, mask)
//   --fill AARRGGBB       initial destination colour (default: a copy of the source)
//   --bench N             repeat prepare + render N times, report the fastest of each
//   --cancel              prepare and one render over the ROI with the cancel flag set
//   --show-params         print the parameter block (32 slots, hex) before running
// Output: one line per run:
//   ok NAME w=W h=H roi=X0,Y0,X1,Y1 tiles=T threads=N prepare_us=P render_us=R mpxs=M
//   err NAME code=C MESSAGE [prepare_us=P render_us=R]
// Exit code 0 when every run succeeded, else 1 (2 = usage).  The assembly version also checks
// guard bytes around every row and that the source is unchanged (codes -200 / -201); render
// writes through fx::Dst (only its ROI) and reads the source through a shared reference, so
// both hold by construction here.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod selftest;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use fx::{Dst, FxArgs, FxDesc, Params, FP_ANGLE, FP_BLOB, FP_BOOL, FP_CHOICE, FP_COLOR, FP_DEC, FP_PAN, FX_NSLOTS};
use gfx::tmask::TMask;
use gfx::{FMT_ARGB, Surface};
use rt::sync::SpinLock;

const MAXTOK: usize = 64;
const DEF_TILE: i32 = 256;
const RAW_MAGIC: u32 = 0x3152_5352; // "RSR1"
const ERR_TRUNC: i32 = -1;
const ERR_HDR: i32 = -2;
const ERR_NOMEM: i32 = -8;
const ERR_DIMS: i32 = -12;
const ERR_IO: i32 = -13;
const ERR_ARGS: i32 = -202;
const ERR_NOFX: i32 = -203;
const ERR_PARAM: i32 = -204;

static TYPENAMES: [&str; 10] = ["?", "int", "dec", "bool", "choice", "angle", "pan", "color", "seed", "blob"];
static BOOLS: [&str; 8] = ["1", "0", "true", "false", "yes", "no", "on", "off"]; // even = true

ui::main!(efftest());

fn efftest() -> i32 {
    fx::use_all();
    img_core::io::set_file_io(img_core::io::FileIo { read: plat::read_file, write: plat::write_file });
    img_all::use_core_codecs();
    let args = plat::args();
    let t = Toks(args.iter().skip(1).take(MAXTOK).map(|s| s.as_str()).collect());
    if t.index("--selftest").is_some() {
        return selftest::run(&t);
    }
    if t.flag("--list") {
        return list(None);
    }
    if let Some(n) = t.value("--props") {
        return list(Some(n));
    }
    if let Some(f) = t.value("--ops") {
        return ops(f);
    }
    if t.value("--fx").is_some() {
        return (run(&t) != 0) as i32;
    }
    plat::write_stderr(b"usage: efftest --shot - (--list | --props NAME | --fx NAME [options] | --ops FILE)\n");
    2
}

pub(crate) fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

// ---------------------------------------------------------------- tokens

pub(crate) struct Toks<'a>(pub Vec<&'a str>);

impl<'a> Toks<'a> {
    /// the index of the first token equal to name
    pub fn index(&self, name: &str) -> Option<usize> {
        self.0.iter().take(MAXTOK).position(|&t| t == name)
    }
    /// the token after `name`
    pub fn value(&self, name: &str) -> Option<&'a str> {
        let i = self.index(name)? + 1;
        if i < self.0.len() && i < MAXTOK { Some(self.0[i]) } else { None }
    }
    pub fn flag(&self, name: &str) -> bool {
        self.index(name).is_some()
    }
    /// parse_int of the token after name, or def
    pub fn int(&self, name: &str, def: i32) -> i32 {
        match self.value(name) {
            Some(v) => parse_int(v.as_bytes()).0,
            None => def,
        }
    }
}

/// parse_int: spaces, sign, decimal digits (wrapping i32) -> (value, end index)  [str.s]
pub(crate) fn parse_int(s: &[u8]) -> (i32, usize) {
    let at = |i: usize| if i < s.len() { s[i] } else { 0 };
    let mut i = 0;
    while at(i) == b' ' {
        i += 1;
    }
    let mut neg = false;
    if at(i) == b'-' {
        neg = true;
        i += 1;
    } else if at(i) == b'+' {
        i += 1;
    }
    let mut v = 0i32;
    while at(i).wrapping_sub(b'0') <= 9 {
        v = v.wrapping_mul(10).wrapping_add((at(i) - b'0') as i32);
        i += 1;
    }
    (if neg { v.wrapping_neg() } else { v }, i)
}

/// parse_hex: optional "#" or "0x", hex digits -> (value, end index)  [str.s]
pub(crate) fn parse_hex(s: &[u8]) -> (u32, usize) {
    let at = |i: usize| if i < s.len() { s[i] } else { 0 };
    let mut i = 0;
    if at(0) == b'#' {
        i = 1;
    } else if at(0) == b'0' && at(1) | 32 == b'x' {
        i = 2;
    }
    let mut v = 0u32;
    loop {
        let d = match hexdig(at(i)) {
            Some(d) => d,
            None => break,
        };
        v = v << 4 | d;
        i += 1;
    }
    (v, i)
}

fn hexdig(c: u8) -> Option<u32> {
    let d = c.wrapping_sub(b'0') as u32;
    if d <= 9 {
        return Some(d);
    }
    let d = (c | 32).wrapping_sub(b'a') as u32;
    if d <= 5 { Some(d + 10) } else { None }
}

// ---------------------------------------------------------------- --ops FILE

fn ops(path: &str) -> i32 {
    let Some(buf) = plat::read_file(path) else {
        out(&format!("err - code=-13 cannot read {path}\n"));
        return 1;
    };
    let mut fails = 0;
    for line in buf.split(|&b| b == b'\n') {
        let toks: Vec<&str> = line
            .split(|&b| b <= 32)
            .filter(|t| !t.is_empty())
            .take(MAXTOK)
            .map(|t| core::str::from_utf8(t).unwrap_or(""))
            .collect();
        if toks.is_empty() || toks[0].starts_with('#') {
            continue;
        }
        if run(&Toks(toks)) != 0 {
            fails += 1;
        }
    }
    (fails != 0) as i32
}

// ---------------------------------------------------------------- --list / --props

fn list(name: Option<&str>) -> i32 {
    let mut found = false;
    for d in (0..).map_while(fx::get) {
        if let Some(n) = name {
            if d.name != n {
                continue;
            }
        }
        found = true;
        show_fx(d);
    }
    if let Some(n) = name {
        if !found {
            out(&format!("err {n} code=-203 unknown effect\n"));
            return 1;
        }
    }
    0
}

fn show_fx(d: &FxDesc) {
    let mut s = format!("effect {} id={} cat={} flags={} nprops={}\n", d.name, d.id, d.category, d.flags, d.props.len());
    for p in d.props {
        let tn = TYPENAMES[if p.typ as usize <= 9 { p.typ as usize } else { 0 }];
        s += &format!("  prop {} type={} slot={} dec={} flags={}", p.name, tn, p.slot, p.dec, p.flags);
        s += &format!(" min={} max={}", base::f64_format(p.min), base::f64_format(p.max));
        if p.typ == FP_COLOR {
            s += &format!(" def={:08X}", p.def_color());
        } else {
            s += &format!(" def={}", base::f64_format(p.def));
            if p.typ == FP_PAN {
                s += &format!(",{}", base::f64_format(p.def_y));
            } else if p.typ == FP_CHOICE && !p.choices.is_empty() {
                s += " choices=";
                s += &p.choices.join("|");
            }
        }
        s += "\n";
    }
    out(&s);
}

// ---------------------------------------------------------------- parameters

/// every "--p key=value" token -> 0 / ERR_PARAM
fn params(d: &FxDesc, t: &Toks, p: &mut Params) -> i32 {
    let n = t.0.len().min(MAXTOK);
    let mut i = 0;
    while i + 1 < n {
        if t.0[i] == "--p" {
            let r = param(d, t.0[i + 1], p);
            if r != 0 {
                return r;
            }
            i += 1;
        }
        i += 1;
    }
    0
}

/// one "key=value" (prints a message on error) -> 0 / ERR_PARAM
fn param(d: &FxDesc, text: &str, p: &mut Params) -> i32 {
    let bad = || {
        out(&format!("err {} code=-204 bad property value {}\n", d.name, text));
        ERR_PARAM
    };
    let tb = text.as_bytes();
    let Some(eq) = tb.iter().position(|&b| b == b'=') else {
        return bad();
    };
    if eq > 63 {
        return bad();
    }
    let key = &text[..eq];
    let val = &tb[eq + 1..];
    let Some(pr) = d.props.iter().find(|pr| pr.name == key) else {
        out(&format!("err {} code=-204 unknown property {}\n", d.name, text));
        return ERR_PARAM;
    };
    let slot = pr.slot as usize;
    if slot >= FX_NSLOTS {
        return bad();
    }
    let parse_i = |p: &mut Params| -> i32 {
        let (v, e) = parse_int(val);
        if e == 0 || e < val.len() {
            return bad();
        }
        p.set_i(slot, v as i64);
        0
    };
    match pr.typ {
        FP_DEC | FP_ANGLE => {
            let (v, e) = base::f64_parse(val);
            if e == 0 || e < val.len() {
                return bad();
            }
            p.set_f(slot, v);
        }
        FP_PAN => {
            if slot >= FX_NSLOTS - 1 {
                return bad();
            }
            let (x, e) = base::f64_parse(val);
            if e == 0 || val.get(e) != Some(&b',') {
                return bad();
            }
            let rest = &val[e + 1..];
            let (y, e2) = base::f64_parse(rest);
            if e2 == 0 || e2 < rest.len() {
                return bad();
            }
            p.set_f(slot, x);
            p.set_f(slot + 1, y);
        }
        FP_COLOR => {
            let mut v = val;
            if v.first() == Some(&b'#') {
                v = &v[1..];
            } else if v.first() == Some(&b'0') && v.get(1).map(|c| c | 32) == Some(b'x') {
                v = &v[2..];
            }
            let (c, e) = parse_hex(v);
            if e == 0 || e < v.len() {
                return bad();
            }
            let c = if e <= 6 { c | 0xFF00_0000 } else { c };
            p.0[slot] = c as u64;
        }
        FP_BOOL => match BOOLS.iter().position(|b| b.as_bytes() == val) {
            Some(k) => p.set_i(slot, (k & 1 ^ 1) as i64),
            None => return parse_i(p),
        },
        FP_CHOICE => {
            if val.first().map(|c| c.wrapping_sub(b'0') <= 9).unwrap_or(false) {
                return parse_i(p);
            }
            match pr.choices.iter().position(|c| c.as_bytes() == val) {
                Some(k) => p.set_i(slot, k as i64),
                None => return bad(),
            }
        }
        FP_BLOB => {
            let n = rt::fx::cvttd(pr.max).min((FX_NSLOTS - slot) as i32);
            if n <= 0 {
                return bad();
            }
            let cap = n as usize * 8;
            for k in 0..cap {
                p.set_byte(slot, k, 0);
            }
            let mut k = 0;
            let mut i = 0;
            while i < val.len() {
                if k >= cap {
                    return bad();
                }
                let (Some(h), Some(l)) = (hexdig(val[i]), val.get(i + 1).and_then(|&c| hexdig(c))) else {
                    return bad();
                };
                p.set_byte(slot, k, (h << 4 | l) as u8);
                k += 1;
                i += 2;
            }
        }
        _ => return parse_i(p),
    }
    0
}

fn show_params(p: &Params) {
    let mut s = String::from("params");
    for v in p.0.iter() {
        s += &format!(" {v:X}");
    }
    s += "\n";
    out(&s);
}

// ---------------------------------------------------------------- images

fn is_raw(path: &str) -> bool {
    let l = path.as_bytes();
    let ends = |e: &[u8]| l.len() >= e.len() && l[l.len() - e.len()..].eq_ignore_ascii_case(e);
    ends(b".raw") || ends(b".argb")
}

/// an FMT_ARGB surface filled with `fill`  [et_alloc]
pub(crate) fn alloc(w: u32, h: u32, fill: u32) -> Result<Surface, i32> {
    if w.wrapping_sub(1) >= 65535 || h.wrapping_sub(1) >= 65535 {
        return Err(ERR_DIMS);
    }
    let mut sf = Surface::new(w as i32, h as i32, FMT_ARGB);
    if fill != 0 {
        let st = sf.stride as usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                fx::put(&mut sf.px[y * st..], x, fill);
            }
        }
    }
    Ok(sf)
}

/// row y of a surface, writable
pub(crate) fn row_mut(sf: &mut Surface, y: i32) -> &mut [u8] {
    let o = y as usize * sf.stride as usize;
    let n = sf.w as usize * 4;
    &mut sf.px[o..o + n]
}

/// raw RSR1 or any image (straight ARGB)  [et_load]
fn load(path: &str) -> Result<Surface, i32> {
    if is_raw(path) {
        let d = plat::read_file(path).ok_or(ERR_IO)?;
        if d.len() < 16 || u32::from_le_bytes([d[0], d[1], d[2], d[3]]) != RAW_MAGIC {
            return Err(ERR_HDR);
        }
        let w = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
        let h = u32::from_le_bytes([d[8], d[9], d[10], d[11]]);
        let mut sf = alloc(w, h, 0)?;
        let rb = w as usize * 4;
        if (rb * h as usize + 16) > d.len() {
            return Err(ERR_TRUNC);
        }
        for y in 0..h as usize {
            row_mut(&mut sf, y as i32).copy_from_slice(&d[16 + y * rb..16 + (y + 1) * rb]);
        }
        return Ok(sf);
    }
    let dec = img_core::ImgDec { flags: img_core::IMGD_STRAIGHT, ..img_core::ImgDec::default() };
    let img = img_core::io::load_ex(path, Some(&dec), None)?;
    let mut sf = alloc(img.w as u32, img.h as u32, 0)?;
    let rb = img.w as usize * 4;
    for y in 0..img.h {
        let o = y as usize * img.stride as usize;
        row_mut(&mut sf, y).copy_from_slice(&img.px[o..o + rb]);
    }
    Ok(sf)
}

/// raw RSR1 (fmt 3) or any image format by extension  [et_save]
fn save(path: &str, sf: &Surface) -> i32 {
    if is_raw(path) {
        let mut b: Vec<u8> = Vec::with_capacity(16 + sf.w as usize * sf.h as usize * 4);
        for v in [RAW_MAGIC, sf.w as u32, sf.h as u32, FMT_ARGB as u32] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for y in 0..sf.h {
            b.extend_from_slice(fx::row(sf, y));
        }
        return if plat::write_file(path, &b) { 0 } else { ERR_IO };
    }
    img_core::io::save_opt(path, sf, None)
}

/// a TMask from the red channel of an image of the source's size  [et_load_mask]
fn load_mask(path: &str, src: &Surface) -> Result<TMask, i32> {
    let img = load(path)?;
    if img.w != src.w || img.h != src.h {
        return Err(ERR_DIMS);
    }
    let (w, h) = (img.w as usize, img.h as usize);
    let mut a8 = vec![0u8; w * h];
    for y in 0..h {
        let r = fx::row(&img, y as i32);
        for x in 0..w {
            a8[y * w + x] = r[x * 4 + 2];
        }
    }
    let mut m = TMask::new(w as u32, h as u32).ok_or(ERR_NOMEM)?;
    let r = m.load_a8(&a8, w);
    if r != 0 {
        return Err(r);
    }
    Ok(m)
}

/// dst = lerp(src, dst, mask) over the whole image  [et_sel_lerp]
pub(crate) fn sel_lerp(src: &Surface, dst: &mut Surface, sel: &TMask) {
    let w = src.w;
    let mut buf = vec![0u8; w as usize];
    for y in 0..src.h {
        let (m, _) = sel.row(0, y, w, &mut buf);
        gfx::straight::lerp_mask_into_b(fx::row(src, y), row_mut(dst, y), m);
    }
}

// ---------------------------------------------------------------- tiles

/// the tile grid of a run  [et_grid]
#[derive(Clone, Copy)]
pub(crate) struct Grid {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    tw: i32,
    th: i32,
    nx: i32,
    pub count: usize,
}

impl Grid {
    pub fn new(d: &FxDesc, roi: [i32; 4], tile: i32, w: i32, h: i32) -> Grid {
        let mut g = Grid { x0: roi[0], y0: roi[1], x1: roi[2], y1: roi[3], tw: tile, th: tile, nx: 1, count: 0 };
        if d.flags & fx::FXF_WHOLE != 0 {
            g = Grid { x0: 0, y0: 0, x1: w, y1: h, tw: w, th: h, ..g };
        } else if d.flags & fx::FXF_BANDS != 0 {
            g = Grid { x0: 0, x1: w, tw: w, ..g };
        }
        let (dx, dy) = (g.x1 - g.x0, g.y1 - g.y0);
        if dx <= 0 || dy <= 0 {
            g.nx = 1;
            return g;
        }
        g.nx = ((dx + g.tw - 1) as u32 / g.tw as u32) as i32;
        let ny = ((dy + g.th - 1) as u32 / g.th as u32) as i32;
        g.count = (ny as u32).wrapping_mul(g.nx as u32) as usize;
        g
    }

    /// every tile
    pub fn rects(&self) -> Vec<[i32; 4]> {
        (0..self.count).map(|i| self.rect(i)).collect()
    }

    /// tile `i` (row-major)  [tile_fn]
    pub fn rect(&self, i: usize) -> [i32; 4] {
        let (ty, tx) = ((i / self.nx as usize) as i32, (i % self.nx as usize) as i32);
        let x0 = tx * self.tw + self.x0;
        let y0 = ty * self.th + self.y0;
        [x0, y0, (x0 + self.tw).min(self.x1), (y0 + self.th).min(self.y1)]
    }
}

/// what every render call of a run shares
pub(crate) struct Run<'a> {
    pub desc: &'a FxDesc,
    pub src: &'a Surface,
    pub params: &'a Params,
    pub aux: Option<&'a (dyn core::any::Any + Send + Sync)>,
    pub sel: Option<&'a TMask>,
}

impl<'a> Run<'a> {
    pub fn args<'b>(&'b self, r: [i32; 4], job: Option<&'b (dyn fx::Cancel + Sync)>) -> FxArgs<'b>
    where
        'a: 'b,
    {
        FxArgs { src: self.src, x0: r[0], y0: r[1], x1: r[2], y1: r[3], params: self.params, aux: self.aux, job, sel: self.sel }
    }

    /// render every tile: serially in order on this thread, or on the pool -> 0 / the first
    /// error  [et_render]
    pub fn render(&self, g: &Grid, dst: &mut Surface, serial: bool) -> i32 {
        let rects = g.rects();
        let mut store = Vec::new();
        let slots = tile_slots(dst, &rects, &mut store);
        self.render_tiles(&rects, &slots, serial)
    }

    /// render the tiles of tile_slots (each slot keeps its destination, so the slots serve
    /// several runs)
    pub fn render_tiles(&self, rects: &[[i32; 4]], slots: &[SpinLock<Option<Dst>>], serial: bool) -> i32 {
        let err = AtomicI32::new(0);
        let one = |i: usize, job: Option<&(dyn fx::Cancel + Sync)>| {
            let mut slot = slots[i].lock();
            let Some(d) = slot.as_mut() else {
                return;
            };
            let e = (self.desc.render)(&self.args(rects[i], job), d);
            if e != 0 {
                let _ = err.compare_exchange(0, e, Ordering::SeqCst, Ordering::SeqCst);
            }
        };
        if serial {
            for i in 0..rects.len() {
                one(i, None);
            }
        } else {
            let item = |i: usize, job: &plat::JobCtl| one(i, Some(job));
            let _ = plat::parallel_for(rects.len(), &item);
        }
        err.load(Ordering::SeqCst)
    }
}

/// the destinations of the tiles (disjoint borrows of dst; the row slices live in store)
pub(crate) fn tile_slots<'a>(dst: &'a mut Surface, rects: &[[i32; 4]], store: &'a mut Vec<&'a mut [u8]>) -> Vec<SpinLock<Option<Dst<'a>>>> {
    fx::tiles(dst, rects, store).into_iter().map(|d| SpinLock::new(Some(d))).collect()
}

/// a cancel flag that is always set (the JOB with JOB_CANCEL of --cancel runs)
pub(crate) static CANCELLED: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------- one run

/// --roi x0,y0,x1,y1 clamped to the source (default: everything)  [et_parse_roi]
fn parse_roi(t: &Toks, w: i32, h: i32) -> [i32; 4] {
    let mut roi = [0, 0, w, h];
    if let Some(v) = t.value("--roi") {
        let b = v.as_bytes();
        let mut pos = 0;
        for r in roi.iter_mut() {
            let (n, e) = parse_int(&b[pos..]);
            *r = n;
            if pos + e >= b.len() {
                break;
            }
            pos += e + 1;
        }
        for (i, r) in roi.iter_mut().enumerate() {
            let lim = if i & 1 != 0 { h } else { w };
            *r = (*r).max(0).min(lim);
        }
        if roi[0] > roi[2] {
            roi[2] = roi[0];
        }
        if roi[1] > roi[3] {
            roi[3] = roi[1];
        }
    }
    roi
}

/// run the effect described by the tokens -> 0 ok / 1
fn run(t: &Toks) -> i32 {
    let name = t.value("--fx").unwrap_or("-");
    let fail = |code: i32, msg: &str| {
        out(&format!("err {name} code={code} {msg}\n"));
        1
    };
    let Some(desc) = fx::find(name) else {
        return fail(ERR_NOFX, "unknown effect");
    };
    let mut p = Params::default();
    fx::defaults(desc, &mut p);
    if params(desc, t, &mut p) != 0 {
        return 1;
    }
    if t.flag("--show-params") {
        show_params(&p);
    }
    let Some(inp) = t.value("--in") else {
        return fail(ERR_ARGS, "missing --in");
    };
    let src = match load(inp) {
        Ok(s) => s,
        Err(e) => return fail(e, "cannot load --in"),
    };
    let fill = t.value("--fill").map(|v| parse_hex(v.as_bytes()).0);
    let mut dst = match alloc(src.w as u32, src.h as u32, fill.unwrap_or(0)) {
        Ok(d) => d,
        Err(e) => return fail(e, "no memory"),
    };
    if fill.is_none() {
        dst.px.copy_from_slice(&src.px);
    }
    let sel = match t.value("--sel") {
        Some(f) => match load_mask(f, &src) {
            Ok(m) => Some(m),
            Err(e) => return fail(e, "cannot load --sel (same size as --in needed)"),
        },
        None => None,
    };
    let roi = parse_roi(t, src.w, src.h);
    let mut tile = t.int("--tile", DEF_TILE);
    if tile < 1 {
        tile = DEF_TILE;
    }
    let serial = t.int("--threads", 0) == 1;
    let reps = t.int("--bench", 1).max(1);
    let cancel = t.flag("--cancel");
    let g = Grid::new(desc, roi, tile, src.w, src.h);
    let threads = if serial { 1 } else { plat::thr_count() + 1 };
    let base = Run { desc, src: &src, params: &p, aux: None, sel: sel.as_ref() };
    if cancel {
        let flag: &(dyn fx::Cancel + Sync) = &CANCELLED;
        let t0 = plat::time_us();
        let (code, pus, rus) = match desc.prepare.map(|f| f(&base.args(roi, Some(flag)))) {
            Some(Err(e)) => (e, plat::time_us() - t0, 0),
            res => {
                let aux = match res {
                    Some(Ok(a)) => a,
                    _ => None,
                };
                let t1 = plat::time_us();
                let run = Run { aux: aux.as_deref(), ..base };
                let mut d = Dst::of_surface(&mut dst, roi);
                let e = (desc.render)(&run.args(roi, Some(flag)), &mut d);
                (e, t1 - t0, plat::time_us() - t1)
            }
        };
        out(&format!("err {name} code={code} cancel prepare_us={pus} render_us={rus}\n"));
        return 1;
    }
    let (mut best_p, mut best_r) = (u64::MAX, u64::MAX);
    // the tile destinations are set up once for all repetitions (safe disjoint borrows of dst;
    // the asm hands raw pointers to its workers)
    let rects = g.rects();
    let mut store = Vec::new();
    let slots = tile_slots(&mut dst, &rects, &mut store);
    for _ in 0..reps {
        let t0 = plat::time_us();
        let aux = match desc.prepare {
            Some(f) => match f(&base.args(roi, None)) {
                Ok(a) => a,
                Err(e) => return fail(e, "prepare failed"),
            },
            None => None,
        };
        let t1 = plat::time_us();
        best_p = best_p.min(t1 - t0);
        let run = Run { aux: aux.as_deref(), ..base };
        let e = run.render_tiles(&rects, &slots, serial);
        best_r = best_r.min(plat::time_us() - t1);
        drop(aux);
        if e != 0 {
            return fail(e, "render failed");
        }
    }
    drop(slots);
    if let Some(m) = &sel {
        sel_lerp(&src, &mut dst, m);
    }
    if let Some(o) = t.value("--out") {
        let e = save(o, &dst);
        if e != 0 {
            return fail(e, "cannot write --out");
        }
    }
    let px = (roi[2] - roi[0]) as u64 * (roi[3] - roi[1]) as u64;
    let mpxs = px / best_r.max(1);
    out(&format!(
        "ok {name} w={} h={} roi={},{},{},{} tiles={} threads={threads} prepare_us={best_p} render_us={best_r} mpxs={mpxs}\n",
        src.w, src.h, roi[0], roi[1], roi[2], roi[3], g.count
    ));
    0
}
