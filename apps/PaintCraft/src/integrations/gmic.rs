//! Standalone G'MIC-Qt bridge. The Qt application owns its filter UI; documents are
//! touched only after successful completion and validation of all output images.
use alloc::{format, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use rt::sync::SpinLock;
use ui::*;
use crate::app::Pn;
use crate::document::model::DocId;

pub(crate) const COMMAND: u32 = 445;
pub(crate) static BUSY: AtomicBool = AtomicBool::new(false);
struct Job { stop: AtomicBool, result: SpinLock<Option<Result<Vec<gfx::Surface>, String>>> }
#[derive(Default)]
pub(crate) struct Host {
    job: Option<Arc<Job>>, worker: Option<plat::Thread>, open: Shared<i32>,
    doc: Option<DocId>, uid: u64, version: u32, mask: Vec<u8>, w: i32, h: i32,
}
impl Drop for Host {
    fn drop(&mut self) {
        if let Some(job) = &self.job { job.stop.store(true, Ordering::Release); }
        if let Some(worker) = self.worker.take() { worker.join(); }
    }
}
pub(crate) fn states(p: &mut Pn, ui: &mut Ui) {
    crate::app::commands::pn_cmd_set(ui, COMMAND, if p.doc.active().is_some() && p.gmic.job.is_none() { 0 } else { CF_DISABLED });
}
pub(crate) fn command(p: &mut Pn, ui: &mut Ui, id: u32, _: u32) -> bool {
    if id != COMMAND { return false; }
    if p.gmic.job.is_none() { start(p, ui); }
    true
}
pub(crate) fn frame(p: &mut Pn, ui: &mut Ui) {
    let result = p.gmic.job.as_ref().and_then(|job| job.result.lock().take());
    if let Some(result) = result {
        let cancelled = p.gmic.job.as_ref().is_some_and(|j| j.stop.load(Ordering::Acquire));
        if let Some(worker) = p.gmic.worker.take() { worker.join(); }
        p.gmic.job = None;
        if !cancelled {
            match result { Ok(images) => commit(p, images), Err(e) => crate::integrations::file_io::fio_error(p, Some("G’MIC-Qt"), &e) }
        }
        BUSY.store(false, Ordering::Release);
        ui.repass();
    }
    let open = p.gmic.open.clone();
    open.set(p.gmic.job.is_some() as i32);
    if ui.window_begin(0x60E1, crate::preferences::localization::tr("G’MIC-Qt"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT | WF_NOCLOSE, None) {
        ui.text(TS_BODY | TX_WRAP, crate::preferences::localization::tr("G’MIC bearbeitet die aktive Ebene in einem eigenen Fenster. Ein gleich großes Ergebnis wird durch die Auswahl übernommen; mehrere oder anders große Ergebnisse öffnen ein neues Bild."));
        if ui.button(0x90120, crate::preferences::localization::tr("Abbrechen")) & EV_ACT != 0 {
            if let Some(job) = &p.gmic.job { job.stop.store(true, Ordering::Release); }
        }
    }
    ui.window_end();
    if p.gmic.job.is_some() { ui.timer(0x5111, 30); }
}
#[cfg(not(any(windows, target_os = "linux")))]
fn start(p: &mut Pn, _: &mut Ui) {
    crate::integrations::file_io::fio_error(p, Some("G’MIC-Qt"), "Die externe G’MIC-Anbindung benötigt die Desktop-Version.");
}
#[cfg(any(windows, target_os = "linux"))]
fn start(p: &mut Pn, ui: &mut Ui) {
    let Some(d) = p.doc.active() else { return };
    let Some(layer) = crate::document::model::doc_cur_layer(d) else { return };
    let (w, h) = (layer.sf.w, layer.sf.h);
    if w as u64 * h as u64 > 100_000_000 { return; }
    let pixels = match img_core::meta::surf_convert(&layer.sf, gfx::FMT_ARGB) { Ok(v) => v, Err(_) => return };
    let mask = crate::document::selection::sel_mask(d).filter(|_| crate::document::selection::sel_active(d));
    p.gmic.mask = (0..h).flat_map(|y| (0..w).map(move |x| mask.map_or(255, |m| m.get(x, y)))).collect();
    p.gmic.doc = Some(d.id); p.gmic.uid = layer.uid; p.gmic.version = d.ver_px; p.gmic.w = w; p.gmic.h = h;
    let program = plat::arg_value("--gmic-qt").or_else(|| plat::env("PAINTCRAFT_GMIC_QT"))
        .unwrap_or_else(|| String::from(if cfg!(windows) { "gmic_qt.exe" } else { "gmic_qt" }));
    let filter = plat::arg_value("--gmic-command");
    let job = Arc::new(Job { stop: AtomicBool::new(false), result: SpinLock::new(None) });
    let j = job.clone();
    let worker = plat::thread_spawn(0, alloc::boxed::Box::new(move || {
        let result = run(&program, filter.as_deref(), pixels, &j);
        *j.result.lock() = Some(result); plat::wake();
    }));
    if worker.is_none() { crate::integrations::file_io::fio_error(p, Some("G’MIC-Qt"), "Der G’MIC-Thread konnte nicht gestartet werden."); return; }
    p.gmic.worker = worker; p.gmic.job = Some(job); BUSY.store(true, Ordering::Release); ui.repass();
}
#[cfg(any(windows, target_os = "linux"))]
struct Temp(String);
#[cfg(any(windows, target_os = "linux"))]
impl Drop for Temp {
    fn drop(&mut self) {
        // Only remove the explicitly named exchange files in this job's directory.
        let mut files = Vec::new();
        plat::dir_list(&self.0, &mut |n, dir, _| {
            if !dir && (n == "input.png" || output_number(n).is_some()) { files.push(format!("{}/{n}", self.0)); }
        });
        for path in files { plat::delete(&path); }
        plat::delete(&self.0); // succeeds only when empty
    }
}
fn output_number(name: &str) -> Option<u32> {
    let s = name.strip_prefix("output-")?.strip_suffix(".png")?;
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) { return None; }
    s.parse().ok()
}
#[cfg(any(windows, target_os = "linux"))]
fn run(program: &str, filter: Option<&str>, pixels: gfx::Surface, job: &Job) -> Result<Vec<gfx::Surface>, String> {
    let fail = |s: &str| String::from(s);
    let root = plat::temp_dir().ok_or_else(|| fail("Kein temporärer Ordner verfügbar."))?;
    // unpredictable name, owner-only directory: other users cannot pre-create or read it
    let folder = format!("{root}/paintcraft-gmic-{:016x}", plat::random_u64());
    if plat::file_exists(&folder) != 0 || !plat::fs::mkdir_private(&folder) { return Err(fail("Der temporäre G’MIC-Ordner ließ sich nicht anlegen.")); }
    let temp = Temp(folder);
    let input = format!("{}/input.png", temp.0);
    let output = format!("{}/output-%l.png", temp.0);
    let png = img_core::io::encode(&pixels, Some(&img_core::ImgOpt { fmt: img_core::IMGF_PNG, ..Default::default() }))
        .map_err(|_| fail("G’MIC-Eingabe konnte nicht kodiert werden."))?;
    drop(pixels);
    if !plat::write_file(&input, &png) { return Err(fail("G’MIC-Eingabe konnte nicht geschrieben werden.")); }
    drop(png);
    let mut args = alloc::vec!["--output", output.as_str()];
    if let Some(filter) = filter { args.extend_from_slice(&["--command", filter, "--apply"]); }
    args.push(&input);
    let mut child = (plat::process::Command { program, args: &args, cwd: None, env: &[] }).spawn()
        .map_err(|e| format!("G’MIC-Qt konnte nicht gestartet werden ({e}). Bitte die Standalone-Version einrichten und ihren Pfad mit --gmic-qt oder PAINTCRAFT_GMIC_QT angeben."))?;
    child.close_stdin();
    let mut errors = Vec::new(); let mut buf = [0u8; 16384];
    loop {
        if job.stop.load(Ordering::Acquire) { return Ok(Vec::new()); }
        let exit = child.try_wait().map_err(|_| fail("G’MIC-Prozess nicht erreichbar."))?;
        // Bound each draining pass so a verbose child cannot starve cancellation.
        for _ in 0..16 { if !matches!(child.read_stdout(&mut buf), Ok(Some(n)) if n > 0) { break; } }
        for _ in 0..16 {
            match child.read_stderr(&mut buf) {
                Ok(Some(n)) if n > 0 => { let take = n.min(8192usize.saturating_sub(errors.len())); errors.extend_from_slice(&buf[..take]); },
                _ => break,
            }
        }
        if let Some(code) = exit {
            if code != 0 { return Err(format!("G’MIC beendet ({code}): {}", String::from_utf8_lossy(&errors))); }
            break;
        }
        plat::sleep_ms(10);
    }
    let mut files = Vec::new();
    plat::dir_list(&temp.0, &mut |name, dir, size| { if !dir { if let Some(n) = output_number(name) { files.push((n, String::from(name), size)); } } });
    files.sort_by_key(|v| v.0);
    if files.len() > 256 { return Err(fail("G’MIC liefert mehr als 256 Ebenen.")); }
    let mut images = Vec::new(); let mut budget = 100_000_000u64;
    for (_, name, size) in files {
        if job.stop.load(Ordering::Acquire) { return Ok(Vec::new()); }
        if budget==0 { return Err(fail("G’MIC-Ergebnis zu groß.")); }
        if size > 512_000_000 { return Err(fail("G’MIC-Ergebnis überschreitet die Dateigrößengrenze.")); }
        let bytes = plat::read_file(&format!("{}/{name}", temp.0)).ok_or_else(|| fail("G’MIC-Ergebnis nicht lesbar."))?;
        let image = img_core::io::decode_ex(&bytes, Some(&img_core::ImgDec { flags: img_core::IMGD_STRAIGHT, maxpx: budget, maxmem: 512_000_000, ..Default::default() }), None)
            .map_err(|_| fail("Ungültiges oder zu großes G’MIC-Ergebnis."))?;
        let n = image.w as u64 * image.h as u64;
        budget = budget.checked_sub(n).ok_or_else(|| fail("G’MIC-Ergebnis zu groß."))?;
        images.push(image);
    }
    Ok(images)
}
fn commit(p: &mut Pn, mut images: Vec<gfx::Surface>) {
    if images.is_empty() { return; } // Qt Cancel produces no files.
    let (w, h) = (p.gmic.w, p.gmic.h);
    if images.len() != 1 || images[0].w != w || images[0].h != h {
        let w = images.iter().map(|s| s.w).max().unwrap_or(1);
        let h = images.iter().map(|s| s.h).max().unwrap_or(1);
        if w as u64 * h as u64 * images.len() as u64 > 100_000_000 { crate::integrations::file_io::fio_error(p, Some("G’MIC-Qt"), "Die Ergebnis-Leinwände überschreiten die Speichergrenze."); return; }
        let Some(id) = crate::document::model::doc_new(p, w, h, 0, crate::app::constants::DNF_EMPTY as u32) else { return; };
        // %l is top-first; PaintCraft stores bottom-first. Pad smaller images with alpha.
        for (i, image) in images.into_iter().rev().enumerate() {
            let Ok(mut sf) = img_core::meta::surf_alloc(w as u32, h as u32, gfx::FMT_ARGB, &Default::default()) else { crate::document::model::doc_close(p, id); return; };
            for y in 0..image.h as usize { let n = image.w as usize * 4; sf.px[y * sf.stride as usize..y * sf.stride as usize+n].copy_from_slice(&image.px[y * image.stride as usize..y * image.stride as usize+n]); }
            if crate::document::layers::lay_add_sf(p, id, i as i32, sf, Some(&format!("G’MIC {}", i+1))).is_err() { crate::document::model::doc_close(p, id); return; }
        }
        p.doc.dm(id).cur = p.doc.d(id).layers.len() as i32 - 1;
        crate::document::model::doc_mark_layers(p, id); crate::document::history::hist_init_unsaved(p, id, "G’MIC importieren", 0);
        crate::document::model::doc_set_modified(p, id, true);
        return;
    }
    let Some(id) = p.gmic.doc else { return; }; let Some(d) = p.doc.get(id) else { return; };
    let Some(li) = crate::document::layers::lay_find_uid(d, p.gmic.uid) else { return; };
    if d.ver_px != p.gmic.version || d.layers[li as usize].sf.w != w || d.layers[li as usize].sf.h != h {
        crate::integrations::file_io::fio_error(p, Some("G’MIC-Qt"), "Das Bild wurde während der Filterausführung geändert. Das Ergebnis wurde verworfen."); return;
    }
    if crate::document::history::hist_px_begin(p, id, li) != 0 { return; }
    if crate::document::history::hist_px_touch(p, id, 0, 0, w, h) != 0 { crate::document::history::hist_px_cancel(p, id); return; }
    let mut result = images.remove(0); let sf = &mut p.doc.dm(id).layers[li as usize].sf;
    for y in 0..h as usize {
        let old = &mut sf.px[y * sf.stride as usize..y * sf.stride as usize+w as usize*4];
        let row = &mut result.px[y * result.stride as usize..y * result.stride as usize+w as usize*4];
        gfx::straight::lerp_mask_into_b(old, row, &p.gmic.mask[y*w as usize..(y+1)*w as usize]); old.copy_from_slice(row);
    }
    crate::document::model::doc_mark_px(p, id, li, 0, 0, w, h); crate::document::history::hist_px_end(p, id, "G’MIC-Qt", 0); crate::document::composite::comp_mark_all(p, id);
}
