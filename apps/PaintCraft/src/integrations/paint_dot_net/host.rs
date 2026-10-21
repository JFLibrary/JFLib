// Classic Paint.NET effects execute in a separate Windows or Wine process. The
// document is changed only after a complete, validated response; cancel/crash is inert.
use crate::app::Pn;
use crate::document::model::DocId;
#[cfg(any(windows, target_os = "linux"))]
use alloc::boxed::Box;
use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use rt::sync::SpinLock;
use ui::*;

pub(crate) const COMMAND: u32 = 444;
pub(crate) static BUSY: AtomicBool = AtomicBool::new(false);
const WINDOW: u32 = 0x60E0;
struct SharedJob {
    stop: AtomicBool,
    result: SpinLock<Option<Result<Vec<u8>, String>>>,
}
pub(crate) struct Host {
    pick: Shared<i32>,
    open: Shared<i32>,
    path: String,
    kind: Option<String>,
    job: Option<Arc<SharedJob>>,
    worker: Option<plat::Thread>,
    doc: Option<DocId>,
    uid: u64,
    version: u32,
    mask: Vec<u8>,
    w: i32,
    h: i32,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            pick: Shared::new(0),
            open: Shared::new(0),
            path: plat::arg_value("--pdn-plugin").unwrap_or_default(),
            kind: None,
            job: None,
            worker: None,
            doc: None,
            uid: 0,
            version: 0,
            mask: Vec::new(),
            w: 0,
            h: 0,
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        if let Some(j) = &self.job {
            j.stop.store(true, Ordering::Release);
        }
        if let Some(w) = self.worker.take() {
            w.join();
        }
    }
}
pub(crate) fn states(p: &mut Pn, ui: &mut Ui) {
    crate::app::commands::pn_cmd_set(
        ui,
        COMMAND,
        if p.doc.active().is_some() && p.pdn.job.is_none() {
            0
        } else {
            CF_DISABLED
        },
    );
}
pub(crate) fn command(p: &mut Pn, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    if id != COMMAND {
        return false;
    }
    if p.pdn.job.is_some() {
        return true;
    }
    p.pdn.kind = None;
    if !available() {
        crate::integrations::file_io::fio_error(
            p,
            Some("Paint.NET-Plugins"),
            "Originale Paint.NET-DLLs benötigen den optionalen .NET-Host. Unter Linux zusätzlich --pdn-wine, --pdn-dotnet und --pdn-install konfigurieren.",
        );
    } else if plat::arg_value("--pdn-plugin").is_some() {
        start(p, ui);
    } else {
        p.pdn.pick.set(1);
        ui.repass();
    }
    true
}
pub(crate) fn open_effect(p: &mut Pn, ui: &mut Ui, path: &str, kind: &str) {
    if p.pdn.job.is_some() {
        return;
    }
    p.pdn.path = String::from(path);
    p.pdn.kind = Some(String::from(kind));
    start(p, ui);
}
pub(crate) fn frame(p: &mut Pn, ui: &mut Ui) {
    let open = p.pdn.pick.clone();
    if ui.filedlg(
        0x90000,
        FD_OPEN,
        &open,
        &mut p.pdn.path,
        4096,
        &["dll=Paint.NET-Plugin"],
    ) & EV_ACT
        != 0
    {
        start(p, ui);
    }
    let result = p.pdn.job.as_ref().and_then(|j| j.result.lock().take());
    if let Some(result) = result {
        if let Some(w) = p.pdn.worker.take() {
            w.join();
        }
        p.pdn.job = None;
        match result {
            Ok(data) if !data.is_empty() => commit(p, &data),
            Ok(_) => {}
            Err(e) => crate::integrations::file_io::fio_error(p, Some("Paint.NET-Plugin"), &e),
        }
        ui.repass();
        BUSY.store(false, Ordering::Release);
    }
    let open = p.pdn.open.clone();
    open.set(p.pdn.job.is_some() as i32);
    if ui.window_begin(
        WINDOW,
        "Paint.NET-Plugin",
        Some(&open),
        WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT | WF_NOCLOSE,
        None,
    ) {
        ui.text(TS_BODY|TX_WRAP,crate::preferences::localization::tr("Das Plugin wird in einem eigenen Prozess ausgeführt. Seine Einstellungen erscheinen in einem separaten Fenster."));
        if ui.button(0x90020, crate::preferences::localization::tr("Abbrechen")) & EV_ACT != 0 {
            if let Some(j) = &p.pdn.job {
                j.stop.store(true, Ordering::Release);
            }
        }
    }
    ui.window_end();
    if p.pdn.job.is_some() {
        ui.timer(0x5110, 30);
    }
}
#[cfg(not(any(windows, target_os = "linux")))]
fn start(_p: &mut Pn, _ui: &mut Ui) {}
#[cfg(any(windows, target_os = "linux"))]
fn start(p: &mut Pn, ui: &mut Ui) {
    let Some(d) = p.doc.active() else { return };
    let Some(layer) = crate::document::model::doc_cur_layer(d) else {
        return;
    };
    let (w, h) = (layer.sf.w, layer.sf.h);
    if (w as u64) * (h as u64) > 100_000_000 {
        crate::integrations::file_io::fio_error(
            p,
            Some("Paint.NET-Plugin"),
            "Das Bild überschreitet die Host-Grenze von 100 Millionen Pixeln.",
        );
        return;
    }
    let mut input = Vec::with_capacity(24 + w as usize * h as usize * 5);
    for n in [
        0x58464350,
        w as u32,
        h as u32,
        crate::workspace::palette::pal_primary(&p.pal),
        crate::workspace::palette::pal_secondary(&p.pal),
        (p.tool.cfg.width as f32).to_bits(),
    ] {
        input.extend_from_slice(&n.to_le_bytes());
    }
    for y in 0..h as usize {
        let o = y * layer.sf.stride as usize;
        input.extend_from_slice(&layer.sf.px[o..o + w as usize * 4]);
    }
    let mask = crate::document::selection::sel_mask(d)
        .filter(|_| crate::document::selection::sel_active(d));
    p.pdn.mask = (0..h)
        .flat_map(|y| (0..w).map(move |x| mask.map_or(255, |m| m.get(x, y))))
        .collect();
    input.extend_from_slice(&p.pdn.mask);
    p.pdn.doc = Some(d.id);
    p.pdn.uid = layer.uid;
    p.pdn.version = d.ver_px;
    p.pdn.w = w;
    p.pdn.h = h;
    let helper = helper_path();
    let path = p.pdn.path.clone();
    let kind = p.pdn.kind.clone().or_else(|| plat::arg_value("--pdn-type"));
    let defaults = plat::args().iter().any(|x| x == "--pdn-defaults");
    let job = Arc::new(SharedJob {
        stop: AtomicBool::new(false),
        result: SpinLock::new(None),
    });
    let worker_job = job.clone();
    let worker = plat::thread_spawn(
        0,
        Box::new(move || {
            let result = run(
                &helper,
                &path,
                kind.as_deref(),
                defaults,
                &input,
                &worker_job,
            );
            *worker_job.result.lock() = Some(result);
            plat::wake();
        }),
    );
    if worker.is_none() {
        crate::integrations::file_io::fio_error(
            p,
            Some("Paint.NET-Plugin"),
            "Der Plugin-Thread konnte nicht gestartet werden.",
        );
        return;
    }
    p.pdn.worker = worker;
    p.pdn.job = Some(job);
    BUSY.store(true, Ordering::Release);
    ui.repass();
}
#[cfg(any(windows, target_os = "linux"))]
fn run(
    helper: &str,
    path: &str,
    kind: Option<&str>,
    defaults: bool,
    input: &[u8],
    job: &SharedJob,
) -> Result<Vec<u8>, String> {
    let mut args = alloc::vec![helper, path];
    if let Some(t) = kind {
        args.push(t);
    }
    if defaults {
        args.push("--defaults");
    }
    let result = exchange(helper, &args[1..], input, input.len(), 0, || {
        job.stop.load(Ordering::Acquire)
    });
    if job.stop.load(Ordering::Acquire) {
        Ok(Vec::new())
    } else {
        result
    }
}
#[cfg(any(windows, target_os = "linux"))]
pub(crate) use super::transport::{exchange, helper_path};

fn available() -> bool {
    #[cfg(any(windows, target_os = "linux"))]
    {
        super::transport::configured()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

fn commit(p: &mut Pn, data: &[u8]) {
    let (w, h) = (p.pdn.w, p.pdn.h);
    let n = w as usize * h as usize * 4;
    if data.len() != 12 + n
        || data[..4] != 0x58464350u32.to_le_bytes()
        || data[4..8] != (w as u32).to_le_bytes()
        || data[8..12] != (h as u32).to_le_bytes()
    {
        crate::integrations::file_io::fio_error(
            p,
            Some("Paint.NET-Plugin"),
            "Ungültiges Ergebnis: Bildgröße oder Protokoll stimmt nicht.",
        );
        return;
    }
    let Some(id) = p.pdn.doc else { return };
    let Some(d) = p.doc.get(id) else { return };
    let Some(li) = crate::document::layers::lay_find_uid(d, p.pdn.uid) else {
        return;
    };
    if d.ver_px != p.pdn.version
        || d.layers[li as usize].sf.w != w
        || d.layers[li as usize].sf.h != h
    {
        crate::integrations::file_io::fio_error(
            p,
            Some("Paint.NET-Plugin"),
            "Das Bild wurde während der Plugin-Ausführung geändert. Das Ergebnis wurde verworfen.",
        );
        return;
    }
    if crate::document::history::hist_px_begin(p, id, li) != 0 {
        return;
    }
    if crate::document::history::hist_px_touch(p, id, 0, 0, w, h) != 0 {
        crate::document::history::hist_px_cancel(p, id);
        return;
    }
    let layer = &mut p.doc.dm(id).layers[li as usize];
    for y in 0..h as usize {
        let old = &mut layer.sf.px
            [y * layer.sf.stride as usize..y * layer.sf.stride as usize + w as usize * 4];
        let mut row = Vec::from(&data[12 + y * w as usize * 4..12 + (y + 1) * w as usize * 4]);
        gfx::straight::lerp_mask_into_b(
            old,
            &mut row,
            &p.pdn.mask[y * w as usize..(y + 1) * w as usize],
        );
        old.copy_from_slice(&row);
    }
    crate::document::model::doc_mark_px(p, id, li, 0, 0, w, h);
    crate::document::history::hist_px_end(p, id, "Paint.NET-Plugin", 0);
    crate::document::composite::comp_mark_all(p, id);
}
