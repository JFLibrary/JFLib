// backdrop -- the background picture of the logical canvas (G1): decoding it, painting it under
// the grid at every zoom, dragging and resizing it, the file dialog, the commands
//
//   Backdrop (Canvas::bd)        the decoded picture (a copy of at most SHOW_MAX pixels per
//                                side, premultiplied) with its mip levels, the copy scaled for
//                                the zoom shown; key() is part of the canvas' version
//   update(n)                    (every frame, before ui.diagram) a new picture is decoded once
//   paint(g, f, n)               (render time, under the grid) its visible part: from the copy
//                                scaled to the zoom (made once per zoom with a bilinear filter
//                                from the nearest mip level that is not smaller, then kept);
//                                during a gesture, or above SCALED_MAX pixels, that level
//                                unfiltered; its opacity, dimmed to DARK_PCT of it in the dark
//                                theme (a white plan would glare)
//   open(n, ui) / chosen(n, ui)  Ansicht › Hintergrundbild …: the file dialog (doc/file.rs);
//                                the file read, checked against jfn_model's limits, decoded and
//                                fitted into the view, one step; it stays unlocked and selected
//   drag(n, d, done) / resize(n, rect, done)   the node's drag and its handle (the aspect
//                                stays; one step each when they end)
//   fit(n) / set_locked(n, on) / remove(n)   An Ansicht anpassen, Sperren, Hintergrund entfernen
//   inspector(n, ui) / project(n, ui)   its card in the inspector: file, size, position, scale,
//                                opacity, Sperren, An Ansicht anpassen, Ersetzen …, Entfernen;
//                                without a picture "Hintergrundbild …"
//   states / command             CMD_BG_*
//
// Decided: the file keeps the picture as it was (PNG, JPEG, BMP or GIF, the codecs every window
// has; WebP would need a codec of its own) up to 8 MiB and 8192 pixels per side and 32 megapixels
// (jfn_model::background); larger ones are refused with their size rather than scaled down
// unasked.  What is drawn is a copy of at most SHOW_MAX pixels per side.

use alloc::rc::Rc;
use alloc::string::{String, ToString};
use core::cell::RefCell;

use gfx::Surface;
use img_core::mip::MipChain;
use jfn_model::background::{self, Background, BgErr};
use ui::*;

use super::paint::{fl, BG_KEY};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::groups as dgroups;
use crate::workspace::inspector::{field_begin, row, rows_begin};

/// the most pixels per side of the copy that is drawn
pub(crate) const SHOW_MAX: i32 = 4096;
/// the most pixels a picture may have (32 megapixels)
pub(crate) const MAX_PIXELS: u64 = background::MAX_PIXELS;
/// the dark theme draws the picture at this much of its opacity (percent)
pub(crate) const DARK_PCT: u32 = 60;
/// a copy scaled for a zoom is kept up to this many pixels
const SCALED_MAX: i64 = 16 << 20;

/// the copy for one zoom
#[derive(Default)]
struct Scaled {
    w: i32,
    h: i32,
    sf: Option<Surface>,
}

/// the decoded picture, its mip levels and the copy for the zoom shown
#[derive(Default)]
pub(crate) struct Backdrop {
    /// the bytes it was decoded from (Rc::as_ptr of the Arc) and their length
    src: (usize, usize),
    pic: Option<Rc<Surface>>,
    mips: Option<MipChain>,
    scaled: RefCell<Scaled>,
    key: u32,
    /// the inspector's fields: x, y, scale (%), opacity (%), and what they came from
    pub fields: [String; 4],
    fields_src: [i64; 5],
    pub err: String,
}

impl Backdrop {
    /// changes with the decoded picture (the canvas' version)
    pub(crate) fn key(&self) -> u32 {
        self.key
    }
    /// a picture is decoded
    pub(crate) fn ready(&self) -> bool {
        self.pic.is_some()
    }
}

/// a picture decoded and made small enough to draw: a copy of at most SHOW_MAX pixels per side
pub(crate) fn decode(data: &[u8]) -> Option<Surface> {
    let dec = img_core::ImgDec { flags: 0, frame: 0, hint: 0, maxpx: MAX_PIXELS, maxmem: 512 << 20, cancel: None };
    let sf = img_core::io::decode_ex(data, Some(&dec), None).ok()?;
    if sf.w <= 0 || sf.h <= 0 {
        return None;
    }
    let side = sf.w.max(sf.h);
    if side <= SHOW_MAX {
        return Some(sf);
    }
    let (w, h) = (((sf.w as i64 * SHOW_MAX as i64) / side as i64).max(1) as i32, ((sf.h as i64 * SHOW_MAX as i64) / side as i64).max(1) as i32);
    let mut dst = Surface::try_new(w, h, gfx::FMT_PARGB)?;
    if img_core::resample::resize(&sf, &mut dst, img_core::resample::RS_BILINEAR, 0) != 0 {
        return None;
    }
    Some(dst)
}

/// (every frame, before ui.diagram) a new picture is decoded once
pub(crate) fn update(n: &mut Net) {
    let src = src_of(n);
    if n.canvas.bd.src == src {
        return;
    }
    let bd = &mut n.canvas.bd;
    let sf = n.doc.proj.background.as_ref().filter(|b| !b.data.is_empty()).and_then(|b| decode(&b.data));
    set_pic(bd, src, sf);
}

// the picture decoded from the bytes src (their address and length), with its mip levels
fn set_pic(bd: &mut Backdrop, src: (usize, usize), sf: Option<Surface>) {
    bd.src = src;
    bd.pic = None;
    bd.mips = None;
    *bd.scaled.borrow_mut() = Scaled::default();
    if let Some(sf) = sf {
        let mut mips = MipChain::alloc(sf.w, sf.h, 0);
        if let Some(m) = mips.as_mut() {
            if m.update(&sf, None) != 0 {
                mips = None;
            }
        }
        bd.pic = Some(Rc::new(sf));
        bd.mips = mips;
    }
    bd.key = (src.0 as u32 ^ (src.1 as u32).rotate_left(16) ^ bd.pic.is_some() as u32).wrapping_mul(0x9E37_79B9);
}

// the address and length of the project's picture bytes
fn src_of(n: &Net) -> (usize, usize) {
    n.doc.proj.background.as_ref().map_or((0, 0), |b| (alloc::sync::Arc::as_ptr(&b.data) as *const u8 as usize, b.data.len()))
}

/// (render time) the picture's visible part under the grid
pub(crate) fn paint(g: &mut Gfx, f: &DgFrame, n: &Net) {
    let (Some(b), Some(pic)) = (n.doc.proj.background.as_ref(), n.canvas.bd.pic.as_ref()) else { return };
    let rect = b.rect();
    if !f.visible(rect) {
        return;
    }
    let q = f.vp.rect_to_screen(rect);
    let (x, y) = (fl(q[0] + 0.5), fl(q[1] + 0.5));
    let (tw, th) = ((fl(q[0] + q[2] + 0.5) - x).max(1), (fl(q[1] + q[3] + 0.5) - y).max(1));
    let pct = b.opacity_pct() * if f.dark { DARK_PCT } else { 100 } / 100;
    let op = (pct * 255 + 50) / 100;
    let saved = g.dc.opac;
    g.dc.set_opacity(op.min(255));
    // the copy for this zoom (made once with a filter from the nearest mip level that is not
    // smaller, or from the copy itself when it grows); during a gesture, or larger than
    // SCALED_MAX, the pixels of that level unfiltered
    let bd = &n.canvas.bd;
    let mut sc = bd.scaled.borrow_mut();
    let fits = (tw as i64) * (th as i64) <= SCALED_MAX;
    if (sc.w != tw || sc.h != th || sc.sf.is_none()) && !f.moving && fits {
        let src = level(bd, pic, tw, th);
        sc.sf = Surface::try_new(tw, th, gfx::FMT_PARGB).and_then(|mut d| (img_core::resample::resize(src, &mut d, img_core::resample::RS_BILINEAR, 0) == 0).then_some(d));
        sc.w = tw;
        sc.h = th;
    }
    match sc.sf.as_ref().filter(|s| s.w == tw && s.h == th) {
        Some(s) => g.dc.blit(g.sf, s, x, y),
        None => g.dc.blit_scaled(g.sf, level(bd, pic, tw, th), x, y, tw, th),
    }
    g.dc.set_opacity(saved);
}

// the smallest mip level (or the copy) at least tw x th
fn level<'a>(bd: &'a Backdrop, pic: &'a Surface, tw: i32, th: i32) -> &'a Surface {
    let mut best: &Surface = pic;
    if let Some(m) = bd.mips.as_ref() {
        for k in 1..=m.n {
            match m.level(k) {
                Some(l) if l.w >= tw && l.h >= th => best = l,
                _ => break,
            }
        }
    }
    best
}

// ---------------------------------------------------------------- the file

/// Ansicht › Hintergrundbild …: the file dialog for a picture (doc/file.rs answers with
/// CMD_BG_CHOSEN)
pub(crate) fn open(n: &mut Net, ui: &mut Ui) {
    let start = crate::doc::file::start_path(n, ui, None);
    let dir = if start.to_lowercase().ends_with(".jfnet") { String::from(start.rsplit_once(['/', '\\']).map_or("", |x| x.0)) } else { start };
    n.file.fd_save = false;
    n.file.fd_image = true;
    n.file.fd_path = if dir.is_empty() { dir } else { alloc::format!("{}/", dir.trim_end_matches(['/', '\\'])) };
    n.file.fd_active = true;
    n.file.fd_open.set(1);
    ui.repass();
}

/// the picture of a file read and checked (jfn_model's limits, decodable) -> it, decoded
pub(crate) fn read(path: &str) -> Result<(Background, Surface), String> {
    let name = crate::doc::file::base_name(path);
    let data = plat::read_file(path).ok_or_else(|| String::from(t("Die Datei ließ sich nicht lesen.")))?;
    let why = |e: BgErr| crate::app::tr::msg(&e.msg());
    if data.len() > background::MAX_BYTES {
        return Err(why(BgErr::TooLarge(data.len())));
    }
    background::sniff(&data).ok_or_else(|| why(BgErr::Format))?;
    let mut meta = img_core::ImgMeta::default();
    img_core::io::probe(&data, Some(&mut meta));
    if meta.w > background::MAX_SIDE || meta.h > background::MAX_SIDE || meta.w as u64 * meta.h as u64 > MAX_PIXELS {
        return Err(why(BgErr::TooWide(meta.w, meta.h)));
    }
    let full = img_core::io::decode_ex(&data, Some(&img_core::ImgDec { flags: 0, frame: 0, hint: 0, maxpx: MAX_PIXELS, maxmem: 512 << 20, cancel: None }), None).map_err(|_| why(BgErr::Broken))?;
    let (w, h) = (full.w as u32, full.h as u32);
    drop(full);
    let bg = Background::new(name, data, w, h).map_err(why)?;
    let sf = decode(&bg.data).ok_or_else(|| why(BgErr::Broken))?;
    Ok((bg, sf))
}

/// the file dialog chose a picture (CMD_BG_CHOSEN): in the middle of the view, fitted into it,
/// unlocked and selected (one step); a refusal says why above the canvas
pub(crate) fn chosen(n: &mut Net, ui: &mut Ui) {
    let path = n.file.fd_path.clone();
    match read(&path) {
        Ok((mut bg, sf)) => {
            fit_into(&mut bg, n.canvas.st.vp.world_rect());
            let label = if n.doc.proj.background.is_some() { "Hintergrundbild ersetzen" } else { "Hintergrundbild einfügen" };
            dgroups::set_bg(&mut n.doc, label, Some(bg));
            // the copy decoded already is the one to draw
            let src = src_of(n);
            set_pic(&mut n.canvas.bd, src, Some(sf));
            n.canvas.st.sel.clear();
            n.canvas.st.sel.insert(BG_KEY);
            n.file.msg = None;
        }
        Err(e) => {
            n.file.msg = Some((IB_ERROR, tf("„{}“ ist kein Hintergrundbild", &[crate::doc::file::base_name(&path)]), e));
        }
    }
    ui.repass();
}

// ---------------------------------------------------------------- on the canvas

/// the node's drag (DEv::Move with BG_KEY): live, one step at its end (the caller groups it
/// with the devices that move along)
pub(crate) fn drag(n: &mut Net, total: [i32; 2], done: bool) {
    let Some(b) = n.doc.proj.background.as_ref() else { return };
    let start = *n.canvas.bg_drag.get_or_insert(b.place());
    dgroups::bg_live(&mut n.doc, |b| {
        b.x = start[0] as i32 + total[0];
        b.y = start[1] as i32 + total[1];
    });
    if done {
        n.canvas.bg_drag = None;
        dgroups::bg_since(&mut n.doc, start, "Hintergrundbild verschieben");
    }
}

/// the node's handle (DEv::Resize): the width sets the scale, the aspect stays (one step)
pub(crate) fn resize(n: &mut Net, rect: [f32; 4], done: bool) {
    let Some(b) = n.doc.proj.background.as_ref() else { return };
    let start = *n.canvas.bg_drag.get_or_insert(b.place());
    let w = b.w.max(1) as f32;
    let pm = ((rect[2] / w * 1000.0 + 0.5) as u32).clamp(background::SCALE_MIN, background::SCALE_MAX);
    dgroups::bg_live(&mut n.doc, |b| b.scale_pm = pm);
    if done {
        n.canvas.bg_drag = None;
        dgroups::bg_since(&mut n.doc, start, "Hintergrundbild skalieren");
    }
}

/// An Ansicht anpassen: centred in the view, fitted into it (one step)
pub(crate) fn fit(n: &mut Net) {
    let Some(mut b) = n.doc.proj.background.clone() else { return };
    fit_into(&mut b, n.canvas.st.vp.world_rect());
    dgroups::bg_place(&mut n.doc, "Hintergrundbild anpassen", |x| x.set_place(b.place()));
}

/// Sperren: a locked picture cannot be selected or dragged (one step)
pub(crate) fn set_locked(n: &mut Net, on: bool) {
    dgroups::bg_place(&mut n.doc, if on { "Hintergrundbild sperren" } else { "Hintergrundbild entsperren" }, |b| b.locked = on);
    if on {
        n.canvas.st.sel.remove(&BG_KEY);
    }
}

/// Hintergrund entfernen (one step)
pub(crate) fn remove(n: &mut Net) {
    dgroups::set_bg(&mut n.doc, "Hintergrund entfernen", None);
    n.canvas.st.sel.remove(&BG_KEY);
}

/// enabled / checked of the picture's commands (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let has = n.doc.proj.background.is_some();
    let locked = n.doc.proj.background.as_ref().is_some_and(|b| b.locked);
    ui.cmd_state(CMD_BG_LOCK, if has { if locked { CF_CHECKED } else { 0 } } else { CF_DISABLED });
    ui.cmd_state(CMD_BG_REMOVE, if has { 0 } else { CF_DISABLED });
    ui.cmd_state(CMD_BG_FIT, if has { 0 } else { CF_DISABLED });
}

/// the picture's commands -> handled
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32) -> bool {
    match id {
        CMD_BG_OPEN => open(n, ui),
        CMD_BG_CHOSEN => chosen(n, ui),
        CMD_BG_LOCK => {
            let on = !n.doc.proj.background.as_ref().is_some_and(|b| b.locked);
            set_locked(n, on);
        }
        CMD_BG_REMOVE => remove(n),
        CMD_BG_FIT => fit(n),
        _ => return false,
    }
    ui.repass();
    true
}

// ---------------------------------------------------------------- the inspector

// "1 200 × 800 Pixel, PNG, 245 KB"
fn size_text(b: &Background) -> String {
    let kb = b.data.len().div_ceil(1024);
    tf("{} × {} Pixel, {}, {} KB", &[&b.w.to_string(), &b.h.to_string(), &b.format.to_uppercase(), &kb.to_string()])
}

// a whole number, or a decimal with the language's mark or a point
fn num(s: &str) -> Option<f64> {
    let t = s.trim().replace(',', ".");
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, String::from(r)),
        None => (false, t),
    };
    let (a, b) = t.split_once('.').unwrap_or((&t, ""));
    if (a.is_empty() && b.is_empty()) || !a.bytes().all(|c| c.is_ascii_digit()) || !b.bytes().all(|c| c.is_ascii_digit()) || a.len() > 7 || b.len() > 3 {
        return None;
    }
    let mut v = a.parse::<f64>().unwrap_or(0.0);
    let mut k = 0.1;
    for c in b.bytes() {
        v += (c - b'0') as f64 * k;
        k *= 0.1;
    }
    Some(if neg { -v } else { v })
}

// a scale or opacity in percent as text ("100", "37,5")
fn pct_text(pm: u32) -> String {
    let sep = if crate::app::tr::en() { '.' } else { ',' };
    if pm % 10 == 0 { (pm / 10).to_string() } else { alloc::format!("{}{}{}", pm / 10, sep, pm % 10) }
}

/// the picture's card in the inspector (the picture selected, or the project's page)
pub(crate) fn inspector(n: &mut Net, ui: &mut Ui) {
    let Some(b) = n.doc.proj.background.clone() else { return };
    ui.text(TS_SUBTITLE, t("Hintergrundbild"));
    rows_begin(ui);
    row(ui, t("Datei"), &b.name);
    row(ui, t("Bild"), &size_text(&b));
    if !n.canvas.bd.ready() {
        row(ui, t("Zustand"), t("lässt sich nicht anzeigen"));
    }
    ui.box_end();
    // position, scale, opacity: Enter or Setzen applies them
    let src = b.place();
    let bd = &mut n.canvas.bd;
    if bd.fields_src != src {
        bd.fields = [b.x.to_string(), b.y.to_string(), pct_text(if b.scale_pm == 0 { 1000 } else { b.scale_pm }), b.opacity_pct().to_string()];
        bd.fields_src = src;
        bd.err.clear();
    }
    let mut set = false;
    for (k, key, id) in [(0, t("Links"), ID_BG_X), (1, t("Oben"), ID_BG_Y), (2, t("Maßstab (%)"), ID_BG_SCALE), (3, t("Deckkraft (%)"), ID_BG_OPACITY)] {
        field_begin(ui, key);
        ui.next_size(-1, 0);
        if ui.textbox(id, None, &mut n.canvas.bd.fields[k], 16) & EV_COMMIT != 0 && ui.focused() == id {
            set = true;
        }
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(ID_BG_SET, t("Setzen")) & EV_ACT != 0 {
        set = true;
    }
    ui.box_end();
    if set {
        let f = n.canvas.bd.fields.clone();
        let (x, y, sc, op) = (num(&f[0]), num(&f[1]), num(&f[2]), num(&f[3]));
        match (x, y, sc, op) {
            (Some(x), Some(y), Some(sc), Some(op)) if (1.0..=10_000.0).contains(&sc) && (5.0..=100.0).contains(&op) && x.abs() < 1.0e7 && y.abs() < 1.0e7 => {
                let pm = (sc * 10.0 + 0.5) as u32;
                dgroups::bg_place(&mut n.doc, "Hintergrundbild einstellen", |b| {
                    b.x = x as i32;
                    b.y = y as i32;
                    b.scale_pm = pm;
                    b.opacity = (op + 0.5) as u32;
                });
                n.canvas.bd.err.clear();
            }
            _ => n.canvas.bd.err = String::from(t("Links und Oben in Einheiten der Fläche, Maßstab 1 bis 10 000 %, Deckkraft 5 bis 100 %.")),
        }
        ui.repass();
    }
    if !n.canvas.bd.err.is_empty() {
        let e = n.canvas.bd.err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
    let mut locked = b.locked as i32;
    if ui.checkbox(ID_BG_LOCK, t("Sperren"), &mut locked) & EV_ACT != 0 {
        set_locked(n, locked != 0);
        ui.repass();
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Gesperrt lässt es sich auf der Zeichenfläche weder wählen noch ziehen; ungesperrt verschiebt Ziehen es, der Griff unten rechts skaliert es."));
    ui.box_begin(0, BOX_H | BOX_WRAP, 8);
    if ui.button(ID_BG_FIT, t("An Ansicht anpassen")) & EV_ACT != 0 {
        fit(n);
        ui.repass();
    }
    if ui.button(ID_BG_REPLACE, t("Ersetzen …")) & EV_ACT != 0 {
        open(n, ui);
    }
    if ui.button(ID_BG_REMOVE, t("Entfernen")) & EV_ACT != 0 {
        remove(n);
        ui.repass();
    }
    ui.box_end();
}

/// the project's page: the picture's card, or "Hintergrundbild …" without one
pub(crate) fn project(n: &mut Net, ui: &mut Ui) {
    if n.doc.proj.background.is_some() {
        inspector(n, ui);
        return;
    }
    ui.box_begin(0, BOX_H, 8);
    if ui.button(ID_BG_ADD, t("Hintergrundbild …")) & EV_ACT != 0 {
        open(n, ui);
    }
    ui.box_end();
}

/// the selected keys hold the picture alone
pub(crate) fn selected(n: &Net) -> bool {
    n.canvas.st.sel.len() == 1 && n.canvas.st.sel.contains(&BG_KEY)
}

/// the place a picture gets in a view (world x0, y0, x1, y1): centred, nine tenths of it at
/// most (scale per mille and the top left corner)
pub(crate) fn fit_into(b: &mut Background, v: [f32; 4]) {
    let (vw, vh) = ((v[2] - v[0]).max(1.0), (v[3] - v[1]).max(1.0));
    let k = (vw * 0.9 / b.w.max(1) as f32).min(vh * 0.9 / b.h.max(1) as f32);
    b.scale_pm = ((k * 1000.0 + 0.5) as u32).clamp(background::SCALE_MIN, background::SCALE_MAX);
    let s = b.scale();
    let (cx, cy) = ((v[0] + v[2]) * 0.5, (v[1] + v[3]) * 0.5);
    b.x = fl(cx - b.w as f32 * s * 0.5 + 0.5);
    b.y = fl(cy - b.h as f32 * s * 0.5 + 0.5);
}
