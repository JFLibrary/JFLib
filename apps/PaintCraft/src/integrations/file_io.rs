// fileio -- PaintCraft: Datei menu.  Neu (dlg_new.rs), Öffnen (system file dialog, every
// registered decoder, .pdn with all layers), Zuletzt geöffnet (in memory),
// Speichern / Speichern unter (format from the extension, "Speichern" question before
// flattening a layered image, .pdn through pdn_encode), Alles speichern, Schließen and
// Beenden with "Änderungen speichern?", the window title and the error bar.
//                                              [apps/PaintCraft/src/fileio.s]
//
// Module contract (app/lifecycle.rs):
//   fio_init()  fio_states()  fio_frame()  fio_cmd(id, arg) -> bool (UIC_CLOSE: true = keep)
// Other modules:
//   fio_infobar()                 the error bar (view.rs shows it above the canvas)
//   fio_open_path(path) -> 0 / ERR_*   open a file (an open one is activated)
//   fio_close_doc(doc)            close with "Änderungen speichern?" when modified
//   fio_error(title, msg)         show msg in the error bar
//   fio_error_text() / fio_title_text()   the message of the error bar ("" = none) / the
//                                 window title
//   fio_prep_opt(doc, opt, meta)  the document's metadata into meta and opt (IOF_EMBED_*) --
//                                 the save dialog encodes with it too
// Speichern unter (and the first save of a document in a format) asks for the options in
// "Speicheroptionen" (dlg_saveopt.rs); OK writes the bytes it encoded.
// Command line: `--open FILE` and plain file arguments open images (several files -> several
// tabs); without any, one "Unbenannt" 800 x 600 white image.
// Questions run as a chain (close one image / exit / save all): each step waits for a modal
// dialog window and continues in fio_cmd.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use rt::sync::SpinLock;

use base::ini::Ini;
use gfx::Surface;
use img_core::{ImgDec, ImgMeta, ImgOpt, ERR_DIMS, ERR_FAIL, ERR_IO, ERR_LIMIT, ERR_NOMEM, ERR_UNSUPPORTED};
use img_core::{IFC_DECODE, IFC_ENCODE, IMGD_EXIF, IMGD_ICC, IMGD_STRAIGHT, IMGD_XMP, IMGF_COUNT, IMGF_PDN, IMGF_PNG, IMR_CM};
use img_core::{IOF_EMBED_EXIF, IOF_EMBED_ICC, IOF_EMBED_XMP};
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, doc_count, doc_find_path, doc_index, doc_set_active, doc_set_modified, Doc, DocId};
use crate::app::formatting::fmt_cap;
use crate::app::commands::pn_cmd_label;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

/// memory budget of one image (ImgDec::maxmem, size of a new image): 50 % of the RAM, at
/// least 1.5 GB (one image at the default pixel limit needs 1 GB plus scratch); 2 GB when
/// the RAM is unknown. The wasm memory is capped at 2 GB, so there it is 1 GB. A small file
/// with many big layers (PDN, PSD) is refused with ERR_LIMIT instead of running out of memory.
pub(crate) fn img_mem_budget() -> u64 {
    #[cfg(target_arch = "wasm32")]
    { 0x4000_0000 }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let t = plat::mem_total();
        if t == 0 { 0x8000_0000 } else { (t / 2).max(0x6000_0000) }
    }
}

const PATH_CAP: usize = 1040;
const EXTS_CAP: usize = 4096;
const FMTS_MAX: usize = 64;
const ERR_CAP: usize = 512;
const TITLE_CAP: usize = 600;
const LABEL_CAP: usize = 1100;
pub(crate) static OPEN_BUSY: AtomicBool = AtomicBool::new(false);

enum LoadedImage {
    Flat(Surface, ImgMeta, u32, bool, bool),
    Pdn(img_pdn::PdnDoc),
    Psd(img_all::psd::document::Document),
}
struct OpenJob {
    path: String,
    cancel: AtomicU32,
    progress: AtomicU32,
    result: SpinLock<Option<Result<LoadedImage, i32>>>,
}
#[derive(Default)]
struct OpenQueue {
    paths: Vec<String>,
    job: Option<Arc<OpenJob>>,
    worker: Option<plat::Thread>,
}
impl Drop for OpenQueue {
    fn drop(&mut self) {
        if let Some(job) = &self.job { job.cancel.store(1, Ordering::Release); }
        if let Some(worker) = self.worker.take() { worker.join(); }
    }
}

// internal commands (fileio range 121..149)
pub(crate) const CMD_FIO_NEWDO: u32 = 121; // "Neu" dialog: OK
const CMD_FIO_ASKED: u32 = 122; // arg = DLG_* of "Änderungen speichern?"
const CMD_FIO_FLAT: u32 = 123; // arg = DLG_* of the flatten question
const CMD_FIO_OPENED: u32 = 124; // the open dialog chose fd_path
const CMD_FIO_SAVED: u32 = 125; // the save dialog chose fd_path
const CMD_FIO_FDCANCEL: u32 = 126; // the save dialog closed without a file
pub(crate) const CMD_FIO_SOPT: u32 = 127; // save options dialog: arg DLG_PRIMARY = OK, else cancel

// dialog windows (A1 uses WINID_DLG0 + 0 .. 7)
const WINID_ASK: u32 = WINID_DLG0 + 1;
const WINID_FLAT: u32 = WINID_DLG0 + 2;
#[cfg(target_arch = "wasm32")]
const WINID_SAVE_PENDING: u32 = WINID_DLG0 + 4;
const ID_FIO_BAR: u32 = ID_FIO_FIRST + 40;

// question chains
const CH_NONE: i32 = 0;
const CH_CLOSE: i32 = 1; // close chain_doc
const CH_EXIT: i32 = 2; // close every image, then quit
const CH_SAVEALL: i32 = 3; // save every modified image

// the app's command line options that take a value (their value is no file to open); the
// library's own options are plat::VALUE_OPTS
static VALUE_OPTS: &[&str] = &[
    "--pdn-plugins", "--gmic-qt", "--gmic-command", "--brush-tip", "--language", "--text-font",
    "--pdn-wine", "--pdn-wine-prefix", "--pdn-dotnet", "--pdn-install", "--brush-presets", "--brush-preset",
    "--config", "--settings-save-at", "--plugins", "--plugin-fuzz", "--crash-dir", "--crash-report",
    "--pdn-plugin", "--pdn-host", "--pdn-type", "--acquire-file", "--update-feed",
];

#[cfg(target_arch = "wasm32")]
struct PendingSave {
    request: u32,
    doc: DocId,
    path: String,
    fmt: u32,
}

/// the state of fileio.s (its g_* globals)
pub(crate) struct Fio {
    opening: OpenQueue,
    #[cfg(target_arch = "wasm32")]
    pending_save: Option<PendingSave>,
    /// g_started: fio_start ran (first fio_states)
    pub(crate) started: bool,
    ask_doc: Option<DocId>,
    chain_doc: Option<DocId>,
    sv_doc: Option<DocId>,
    sa_doc: Option<DocId>,
    chain: i32,
    sv_fmt: u32,
    /// g_sv_saveas: the path came from Speichern unter
    sv_saveas: bool,
    ask_open: Shared<i32>,
    ask_pending: bool,
    flat_open: Shared<i32>,
    flat_pending: bool,
    fdo_open: Shared<i32>,
    fds_open: Shared<i32>,
    fds_pending: bool,
    fds_flags: u32,
    ask_rect: Shared<WinRect>,
    flat_rect: Shared<WinRect>,
    /// g_save_fmts: the IMGF_* of each entry of save_exts
    save_fmts: Vec<u32>,
    fd_path: String,
    last_dir: String,
    sv_path: String,
    err_title: String,
    err_msg: String,
    title: String,
    /// web: the page was told that a document has unsaved changes (it then asks before it is closed)
    #[cfg(target_arch = "wasm32")]
    unsaved: bool,
    /// g_recent: newest first
    recent: Vec<String>,
    /// g_open_exts / g_save_exts: the type lists of the file dialogs
    open_exts: Vec<String>,
    save_exts: Vec<String>,
    /// the renderer's copy of the asked document's thumbnail (doc, thumb_aux, copy)
    thumb: Option<(DocId, u32, Rc<Surface>)>,
}

impl Default for Fio {
    fn default() -> Fio {
        Fio {
            opening: OpenQueue::default(),
            #[cfg(target_arch = "wasm32")]
            pending_save: None,
            started: false,
            ask_doc: None,
            chain_doc: None,
            sv_doc: None,
            sa_doc: None,
            chain: CH_NONE,
            sv_fmt: 0,
            sv_saveas: false,
            ask_open: Shared::new(0),
            ask_pending: false,
            flat_open: Shared::new(0),
            flat_pending: false,
            fdo_open: Shared::new(0),
            fds_open: Shared::new(0),
            fds_pending: false,
            fds_flags: FD_SAVE,
            ask_rect: Shared::new(WinRect::new(0, 0, 440, 0, 0)),
            flat_rect: Shared::new(WinRect::new(0, 0, 440, 0, 0)),
            save_fmts: Vec::new(),
            fd_path: String::new(),
            last_dir: String::new(),
            sv_path: String::new(),
            err_title: String::new(),
            err_msg: String::new(),
            title: String::new(),
            #[cfg(target_arch = "wasm32")]
            unsaved: false,
            recent: Vec::new(),
            open_exts: Vec::new(),
            save_exts: Vec::new(),
            thumb: None,
        }
    }
}

// ------------------------------------------------------------------ init

/// fio_init() -- codecs and file type lists (the files of the command line are opened by the
/// first fio_states: every module is initialised then)
pub(crate) fn fio_init(p: &mut Pn, _ui: &mut Ui) {
    img_all::use_all_codecs();
    // jxl-module: JPEG XL is saved by the writer module next to the exe (jxlenc.dll / .so,
    // loaded on the first JPEG XL save); --jxlenc=FILE another file, --jxlenc=off none (then
    // the assembly-compatible writer; the save dialog says so)
    let jxlenc = plat::args().iter().skip(1).find_map(|a| a.strip_prefix("--jxlenc=").map(String::from));
    img_jxl::writer::use_module(jxlenc.as_deref());
    build_filters(&mut p.fio);
    let f = &mut p.fio;
    f.recent.clear();
    f.chain = CH_NONE;
    f.started = false;
    f.last_dir = cap(slashes(&plat::cwd()), PATH_CAP);
}

// cap(s, n) -- str_copy into a buffer of n bytes
fn cap(s: String, n: usize) -> String {
    fmt_cap(s, n)
}

// slashes(path) -- '\' -> '/'
fn slashes(s: &str) -> String {
    s.replace('\\', "/")
}

// set_last_dir(path) -- the folder of path becomes the start folder of the file dialogs
fn set_last_dir(f: &mut Fio, path: &str) {
    let mut d = cap(slashes(path), PATH_CAP);
    if let Some(k) = d.rfind('/') {
        d.truncate(k);
    }
    f.last_dir = d;
}

// dir_path(name) -- fd_path = last_dir + "/" + name (name may be "")
fn dir_path(f: &mut Fio, name: &str) {
    let mut s = f.last_dir.clone();
    if !s.is_empty() {
        s = cap(s + "/", PATH_CAP);
    }
    s.push_str(name);
    f.fd_path = cap(s, PATH_CAP);
}

// fio_start() -- the images a session end kept, then command line files (--open FILE, plain
// arguments), else one new image
fn fio_start(p: &mut Pn, ui: &mut Ui) {
    // first: a command line file that was recovered is then found open (with its changes)
    fio_recover(p, ui);
    let a = plat::args();
    fio_open_args(p, ui, "", a.get(1..).unwrap_or(&[]));
    if doc_count(&p.doc) == 0 {
        new_default(p);
    }
}

// fio_open_args(cwd, args) -- the files of a command line (--open FILE, plain arguments;
// options with a value skip it); relative names against cwd ("" = this process's folder)
fn fio_open_args(p: &mut Pn, ui: &mut Ui, cwd: &str, a: &[String]) {
    let full = |f: &str| {
        let b = f.as_bytes();
        let absolute = cwd.is_empty() || f.contains("://") || b.first().is_some_and(|&c| c == b'/' || c == b'\\')
            || (b.len() >= 2 && b[1] == b':');
        if absolute { String::from(f) } else { format!("{}/{}", cwd.trim_end_matches(['/', '\\']), f) }
    };
    let mut i = 0;
    let mut opts = true; // "--" ends the options: every later argument is a file
    while i < a.len() {
        let s = a[i].clone();
        i += 1;
        if opts && s == "--" {
            opts = false;
            continue;
        }
        if opts && s.starts_with('-') {
            if s == "--open" {
                let Some(f) = a.get(i).cloned() else { break };
                i += 1;
                fio_open_path(p, ui, &full(&f));
            } else if VALUE_OPTS.contains(&s.as_str()) || plat::VALUE_OPTS.contains(&s.as_str()) {
                i += 1; // options with a value: skip the value
            }
            continue;
        }
        fio_open_path(p, ui, &full(&s));
    }
}

// new_default() -- "Unbenannt": 800 x 600, white, 96 dpi
fn new_default(p: &mut Pn) {
    if let Some(d) = crate::document::model::doc_new(p, DEF_W, DEF_H, 0xFFFF_FFFF, 0) {
        crate::document::history::hist_init(p, d, "Neues Bild", IC_PAGE);
    }
}

// build_filters() -- open_exts ("Alle Bildtypen" + every decodable type) and save_exts
// (every encodable type, save_fmts[i] = its IMGF_*) from the format table
fn build_filters(f: &mut Fio) {
    // append_str: n bytes fit when len + n + 2 <= cap
    let fits = |len: usize, n: usize, cap: usize| len + n + 2 <= cap;
    let (mut olen, mut slen, mut alen) = (0usize, 0usize, 0usize);
    let mut body: Vec<String> = Vec::new();
    let mut all = String::new();
    f.save_fmts.clear();
    f.save_exts.clear();
    for fmt in 1..IMGF_COUNT {
        let Some(fi) = img_core::io::format_info(fmt) else { continue };
        if fi.exts.is_empty() {
            continue;
        }
        let n = fi.exts.len(); // entry length
        if fi.caps & IFC_DECODE != 0 {
            let extensions=if fmt==img_core::IMGF_PSD { "psd;psb=Photoshop PSD/PSB" } else { fi.exts };
            let n=extensions.len();
            // the decodable entry, and its extensions for "Alle Bildtypen"
            if fits(olen, n + 1, EXTS_CAP - 64) {
                olen += n + 1;
                body.push(String::from(extensions));
            }
            let e = match extensions.find('=') {
                Some(k) => &extensions[..k],
                None => extensions,
            };
            if alen != 0 && fits(alen, 1, EXTS_CAP - 64) {
                alen += 1;
                all.push(';');
            }
            if fits(alen, e.len(), EXTS_CAP - 64) {
                alen += e.len();
                all.push_str(e);
            }
        }
        if fi.caps & IFC_ENCODE != 0 && f.save_fmts.len() < FMTS_MAX && fits(slen, n + 1, EXTS_CAP) {
            slen += n + 1;
            f.save_exts.push(String::from(fi.exts));
            f.save_fmts.push(fmt);
        }
    }
    // open_exts = the all types entry + body
    if fits(alen, 16, EXTS_CAP - 32) {
        all.push_str("=Alle Bildtypen");
        alen += 16;
    }
    let mut list = alloc::vec![all];
    if alen + olen < EXTS_CAP * 2 - 2 {
        list.extend(body);
    }
    f.open_exts = list;
}

// ------------------------------------------------------------------ helpers

/// fio_error(title, msg) -- the error bar shows `msg` (title None = "Fehler")
pub(crate) fn fio_error(p: &mut Pn, title: Option<&str>, msg: &str) {
    p.fio.err_title = cap(String::from(title.unwrap_or("Fehler")), ERR_CAP);
    p.fio.err_msg = cap(String::from(msg), ERR_CAP);
    p.repass();
}

/// fio_error_text() -> the message of the error bar ("" = none)
pub(crate) fn fio_error_text(f: &Fio) -> &str {
    &f.err_msg
}

/// fio_title_text() -> the window title set last
pub(crate) fn fio_title_text(f: &Fio) -> &str {
    &f.title
}

/// fio_infobar() -- error bar (only while there is an error)
pub(crate) fn fio_infobar(p: &mut Pn, ui: &mut Ui) {
    if p.fio.err_msg.is_empty() {
        return;
    }
    let f = &mut p.fio;
    if ui.infobar(ID_FIO_BAR, IB_ERROR, &f.err_title, &f.err_msg) & EV_ACT != 0 {
        f.err_msg.clear();
        ui.repass();
    }
}

// encodable(fmt) -> true when a registered encoder writes fmt
fn encodable(fmt: u32) -> bool {
    fmt != 0 && img_core::io::format_info(fmt).is_some_and(|fi| fi.caps & IFC_ENCODE != 0)
}

// load_error_msg(err, path) -- German message for a failed open
fn load_error_msg(p: &mut Pn, err: i32, path: &str) {
    let msg = match err {
        ERR_IO => {
            if plat::file_exists(path) != 0 {
                "Die Datei ließ sich nicht vom Datenträger lesen."
            } else {
                "Datei nicht gefunden."
            }
        }
        ERR_UNSUPPORTED => "Dieses Bildformat wird nicht erkannt; die Datei lässt sich nicht öffnen.",
        ERR_NOMEM | ERR_LIMIT => "Zu wenig Arbeitsspeicher, um das Bild zu laden.",
        ERR_DIMS => "Das Bild ist zu groß.",
        ERR_FAIL => "Es sind zu viele Bilder geöffnet.",
        _ => "Die Datei ist defekt oder passt nicht zu ihrem Format.",
    };
    let t = String::from(file_part(path));
    fio_error(p, Some(&t), msg);
}

// file_part(path) -> the file name inside path
fn file_part(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(k) => &path[k + 1..],
        None => path,
    }
}

// canon_path(src) -> absolute path with '/' separators and without "." / ".." segments (a
// relative src starts in the current directory); "" when the path exceeds PATH_LONG (never a
// silently shortened path, which would name another file)
const PATH_LONG: usize = 32768;
fn canon_path(src: &str) -> String {
    let s = src.as_bytes();
    let abs = matches!(s.first(), None | Some(b'/') | Some(b'\\')) || s.get(1) == Some(&b':');
    let t = if abs { String::from(src) } else { format!("{}/{}", plat::cwd(), src) };
    if t.len() > PATH_LONG {
        return String::new();
    }
    let mut b = t.replace('\\', "/").into_bytes();
    // the root that stays: "C:/", "/", "//"
    let mut p = 0;
    if b.get(1) == Some(&b':') {
        p = 2;
    }
    if b.get(p) == Some(&b'/') {
        p += 1;
        if b.get(p) == Some(&b'/') {
            p += 1; // "//server"
        }
    }
    let n = b.len();
    let (mut w, mut r) = (p, p);
    while r < n {
        // segment r .. e
        let mut e = r;
        while e < n && b[e] != b'/' {
            e += 1;
        }
        let len = e - r;
        if len == 0 || (len == 1 && b[r] == b'.') {
            // skipped
        } else if len == 2 && b[r] == b'.' && b[r + 1] == b'.' {
            while w > p {
                w -= 1;
                if b[w] == b'/' {
                    break;
                }
            }
        } else {
            if w > p {
                b[w] = b'/';
                w += 1;
            }
            for k in r..e {
                b[w] = b[k];
                w += 1;
            }
        }
        r = e;
        if r < n && b[r] == b'/' {
            r += 1;
        }
    }
    b.truncate(w);
    String::from_utf8(b).unwrap_or_default()
}

// ------------------------------------------------------------------ open

/// fio_open_path(path) -> 0 / ERR_* (errors are shown in the bar)
pub(crate) fn fio_open_path(p: &mut Pn, ui: &mut Ui, path: &str) -> i32 {
    let path = canon_path(path);
    if let Some(d) = doc_find_path(&p.doc, &path) {
        doc_set_active(p, d);
        return 0;
    }
    let err = 'e: {
        if doc_count(&p.doc) >= DOC_MAX {
            break 'e ERR_FAIL;
        }
        let Some(data) = plat::read_file(&path) else { break 'e ERR_IO };
        match decode_open(&data, &path, None) {
            Ok(image) => install_open(p, ui, image, &path),
            Err(e) => e,
        }
    };
    if err != 0 {
        load_error_msg(p, err, &path);
    }
    err
}

fn decode_open(data: &[u8], path: &str, cancel: Option<&AtomicU32>) -> Result<LoadedImage, i32> {
    let hint = img_core::io::format_from_path(path);
    let dec = ImgDec { flags: IMGD_STRAIGHT | IMGD_ICC | IMGD_EXIF | IMGD_XMP, hint, cancel, maxmem: img_mem_budget(), ..ImgDec::default() };
    if dec.cancelled() { return Err(img_core::ERR_CANCEL); }
    if data.starts_with(b"8BPS") {
        match img_all::psd::document::decode(data, &dec) {
            Ok(Some(document)) => return Ok(LoadedImage::Psd(document)),
            Ok(None) | Err(ERR_UNSUPPORTED) => {},
            Err(e) => return Err(e),
        }
    }
    if img_core::io::detect(data, hint) == IMGF_PDN {
        match img_pdn::decode(data, Some(&dec)) {
            Ok(pd) => return Ok(LoadedImage::Pdn(pd)),
            Err(ERR_UNSUPPORTED) => {},
            Err(e) => return Err(e),
        }
    }
    let layered = data.starts_with(b"8BPS") && (img_all::psd::has_layers(data) || data.get(5)==Some(&2));
    let mut meta = ImgMeta::default();
    let sf = img_core::io::decode_ex(data, Some(&dec), Some(&mut meta))?;
    // (svg) IMGF_SVG lies above the external slots but is a built-in format
    Ok(LoadedImage::Flat(sf, meta, hint, layered, hint >= img_core::IMG_EXTERNAL0 && hint != img_core::IMGF_SVG))
}

fn install_open(p: &mut Pn, ui: &mut Ui, image: LoadedImage, path: &str) -> i32 {
    if doc_count(&p.doc) >= DOC_MAX { return ERR_FAIL; }
    match image {
        LoadedImage::Pdn(pd) => install_pdn(p, ui, pd, path),
        LoadedImage::Psd(psd) => install_psd(p, ui, psd, path),
        LoadedImage::Flat(sf, meta, hint, layered, external) => {
            let high_depth = meta.bits > 8 || meta.flags & (img_core::IMF_HDR | img_core::IMF_FLOAT) != 0;
            let result = install_flat(p, ui, sf, meta, if layered || external || high_depth { "" } else { path }, hint);
            if result == 0 && layered {
                fio_error(p,Some("PSD-Import"),"Das gespeicherte PSD-Gesamtbild wurde als neues 8-Bit-Bild importiert. Nicht unterstützte Photoshop-Strukturen und höhere Farbtiefen bleiben in der Originaldatei erhalten. Bitte unter einem neuen Namen speichern.");
            } else if result == 0 && high_depth {
                fio_error(p,Some("Farbtiefe / HDR"),"Die Quelldatei wurde in ein neues 8-Bit-Dokument umgewandelt. PaintCraft bearbeitet und zeigt derzeit SDR; höhere Präzision und HDR-Werte bleiben nur in der Originaldatei erhalten. Bitte unter einem neuen Namen speichern.");
            }
            if result == 0 && external {
                fio_error(p,Some("Paint.NET-Dateiformat"),"Als neues Bild geöffnet: nur die vereinten Pixel und die Auflösung. Ebenen und übrige Metadaten stehen weiter nur in der Originaldatei; beim Export nutzt das Plugin seine Vorgaben.");
            }
            result
        }
    }
}

/// Interactive open: serialize reads/decodes off the UI thread; commit complete results only.
/// Startup uses fio_open_path before normal interaction. Scalar WebAssembly has no worker threads.
pub(crate) fn fio_queue_path(p: &mut Pn, ui: &mut Ui, path: &str) {
    #[cfg(target_arch = "wasm32")]
    { fio_open_path(p, ui, path); return; }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = canon_path(path);
        let existing = doc_find_path(&p.doc, &path);
        if p.fio.opening.job.is_none() && p.fio.opening.paths.is_empty() {
            if let Some(d) = existing { doc_set_active(p, d); return; }
        }
        let q = &mut p.fio.opening;
        if q.paths.contains(&path) || q.job.as_ref().is_some_and(|j| j.path == path) { return; }
        if existing.is_none() && doc_count(&p.doc) + q.paths.len() as i32 + q.job.is_some() as i32 >= DOC_MAX {
            load_error_msg(p, ERR_FAIL, &path); return;
        }
        q.paths.push(path);
        OPEN_BUSY.store(true, Ordering::Release);
        poll_open(p, ui);
        ui.repass();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_job(job: &OpenJob) -> Result<LoadedImage, i32> {
    let file = plat::File::open(&job.path, plat::FO_READ).ok_or(ERR_IO)?;
    let size = usize::try_from(file.size().ok_or(ERR_IO)?).map_err(|_| ERR_LIMIT)?;
    let mut data = Vec::new();
    data.try_reserve_exact(size).map_err(|_| ERR_NOMEM)?;
    // Check cancellation between bounded reads instead of filling the whole buffer first.
    let mut buffer = alloc::vec![0; 1 << 20];
    while data.len() < size {
        if job.cancel.load(Ordering::Acquire) != 0 { return Err(img_core::ERR_CANCEL); }
        let n = buffer.len().min(size - data.len());
        let read = file.read_at(data.len() as u64, &mut buffer[..n]).filter(|&n| n != 0).ok_or(ERR_IO)?;
        data.extend_from_slice(&buffer[..read]);
        job.progress.store((data.len() as u64 * 100 / size.max(1) as u64) as u32, Ordering::Release);
    }
    job.progress.store(100, Ordering::Release);
    decode_open(&data, &job.path, Some(&job.cancel))
}

fn poll_open(p: &mut Pn, ui: &mut Ui) {
    let result = p.fio.opening.job.as_ref().and_then(|j| j.result.lock().take());
    if let Some(result) = result {
        if let Some(worker) = p.fio.opening.worker.take() { worker.join(); }
        let job = p.fio.opening.job.take().unwrap();
        if job.cancel.load(Ordering::Acquire) == 0 {
            let err = match result {
                Ok(image) => {
                    if let Some(d) = doc_find_path(&p.doc, &job.path) { doc_set_active(p, d); 0 }
                    else { install_open(p, ui, image, &job.path) }
                },
                Err(e) => e,
            };
            if err != 0 && err != img_core::ERR_CANCEL { load_error_msg(p, err, &job.path); }
        }
        ui.repass();
    }
    #[cfg(not(target_arch = "wasm32"))]
    while p.fio.opening.job.is_none() && !p.fio.opening.paths.is_empty() {
        let path = p.fio.opening.paths.remove(0);
        if let Some(d) = doc_find_path(&p.doc, &path) { doc_set_active(p, d); continue; }
        let job = Arc::new(OpenJob { path, cancel: AtomicU32::new(0), progress: AtomicU32::new(0), result: SpinLock::new(None) });
        let work = job.clone();
        let worker = plat::thread_spawn(0, alloc::boxed::Box::new(move || {
            let result = load_job(&work);
            *work.result.lock() = Some(result);
            plat::wake();
        }));
        if worker.is_some() { p.fio.opening.job = Some(job); p.fio.opening.worker = worker; }
        else {
            p.fio.opening.paths.clear();
            fio_error(p, Some("Öffnen"), "Der Hintergrund-Thread konnte nicht gestartet werden.");
        }
    }
    OPEN_BUSY.store(p.fio.opening.job.is_some() || !p.fio.opening.paths.is_empty(), Ordering::Release);
}

fn open_window(p: &mut Pn, ui: &mut Ui) {
    poll_open(p, ui);
    let Some(job) = p.fio.opening.job.as_ref() else { return; };
    ui.timer(0x5112, 30);
    if ui.window_begin(0x60EA, crate::preferences::localization::tr("Bild öffnen"), None, WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT | WF_NOCLOSE, None) {
        ui.text(TS_BODY | TX_WRAP, file_part(&job.path));
        let progress = job.progress.load(Ordering::Acquire);
        ui.text(TS_BODY, &if progress == 100 { String::from("Bild wird dekodiert …") } else { format!("Datei wird gelesen: {progress} %") });
        if ui.button(0x94000, crate::preferences::localization::tr("Abbrechen")) & EV_ACT != 0 {
            job.cancel.store(1, Ordering::Release);
            p.fio.opening.paths.clear();
        }
    }
    ui.window_end();
}

// opened(doc, path, fmt) -- common end of a successful open: path, format, save options,
// unmodified, first history entry, recent files
fn opened(p: &mut Pn, ui: &mut Ui, doc: DocId, path: &str, fmt: u32) {
    {
        let d = p.doc.dm(doc);
        d.fmt = fmt;
        d.saveopt = img_core::io::opt_init(fmt);
    }
    crate::document::model::doc_set_path(p, doc, path);
    recent_add(p, ui, path);
    set_last_dir(&mut p.fio, path);
    crate::document::history::hist_init(p, doc, "Datei öffnen", IC_FOLDER_OPEN);
    doc_set_modified(p, doc, false);
    doc_set_active(p, doc);
}

/// largest resolution taken from a file (dots per inch)
const DPI_MAX: f64 = 100_000.0;

// set_dpi(doc, dpix_16_16, dpiy_16_16, unit) -- resolution from file metadata (0 = keep 96)
fn set_dpi(d: &mut Doc, dpix: u32, dpiy: u32, unit: u32) {
    if dpix == 0 {
        return;
    }
    let dpiy = if dpiy == 0 { dpix } else { dpiy };
    let mut x = (dpix as f64 / 65536.0).min(DPI_MAX);
    let mut y = (dpiy as f64 / 65536.0).min(DPI_MAX);
    d.dpu_unit = 1;
    if unit == IMR_CM {
        d.dpu_unit = 2;
        x /= 2.54;
        y /= 2.54;
    }
    d.dpu_x = x;
    d.dpu_y = y;
}

// open_flat(data, path, hint) -> 0 / ERR_* -- one layer "Hintergrund"
fn open_flat(p: &mut Pn, ui: &mut Ui, data: &[u8], path: &str, hint: u32) -> i32 {
    let dec = ImgDec { flags: IMGD_STRAIGHT | IMGD_ICC | IMGD_EXIF | IMGD_XMP, hint, maxmem: img_mem_budget(), ..ImgDec::default() };
    let mut meta = ImgMeta::default();
    let sf = match img_core::io::decode_ex(data, Some(&dec), Some(&mut meta)) {
        Ok(sf) => sf,
        Err(e) => return e,
    };
    install_flat(p, ui, sf, meta, path, hint)
}

fn install_flat(p: &mut Pn, ui: &mut Ui, sf: Surface, mut meta: ImgMeta, path: &str, hint: u32) -> i32 {
    let Some(doc) = crate::document::model::doc_new(p, sf.w, sf.h, 0, DNF_EMPTY as u32) else { return ERR_NOMEM };
    let li = match crate::document::layers::lay_add_sf(p, doc, 0, sf, Some("Hintergrund")) {
        Ok(i) => i,
        Err(_) => {
            crate::document::model::doc_close(p, doc);
            return ERR_NOMEM;
        }
    };
    let d = p.doc.dm(doc);
    d.layers[li as usize].flags |= LYF_BG as u32;
    // resolution, ICC, EXIF / XMP
    set_dpi(d, meta.dpix, meta.dpiy, meta.resunit);
    d.icc = core::mem::take(&mut meta.icc);
    if !meta.exif.is_empty() || !meta.xmp.is_empty() {
        d.meta = Some(alloc::boxed::Box::new(meta.clone()));
    }
    let fmt = if meta.fmt != 0 { meta.fmt } else { hint };
    if path.is_empty() {
        crate::document::history::hist_init_unsaved(p, doc, "Bilderfassung", IC_SCAN);
        doc_set_active(p,doc);
    } else {opened(p, ui, doc, path, fmt);}
    0
}

pub(crate) fn fio_acquired(p: &mut Pn, ui: &mut Ui, data: &[u8]) -> i32 {
    if doc_count(&p.doc)>=DOC_MAX {return ERR_LIMIT;}
    open_flat(p,ui,data,"",0)
}

// open_pdn(data, path) -> 0 / ERR_* -- every layer with its properties
fn install_pdn(p: &mut Pn, ui: &mut Ui, pd: img_pdn::PdnDoc, path: &str) -> i32 {
    let Some(doc) = crate::document::model::doc_new(p, pd.w as i32, pd.h as i32, 0, DNF_EMPTY as u32) else { return ERR_NOMEM };
    let (unit, dpux, dpuy) = (pd.unit, pd.dpux, pd.dpuy);
    for (i, pl) in pd.layers.into_iter().enumerate() {
        let name = String::from_utf8_lossy(&pl.name).into_owned();
        let li = match crate::document::layers::lay_add_sf(p, doc, i as i32, pl.sf, Some(&name)) {
            Ok(li) => li,
            Err(_) => {
                crate::document::model::doc_close(p, doc);
                return ERR_NOMEM;
            }
        };
        let l = &mut p.doc.dm(doc).layers[li as usize];
        l.sf.opac = pl.opacity;
        l.sf.blend = if pl.blend as u32 <= gfx::blendmode::BM_XOR { pl.blend } else { 0 };
        let mut fl = if pl.visible != 0 { LYF_VISIBLE as u32 } else { 0 };
        if pl.isbg != 0 {
            fl |= LYF_BG as u32;
        }
        l.flags = fl;
        l.meta = pl.meta;
    }
    // resolution (paint.net MeasurementUnit)
    let u = match unit {
        img_pdn::PDU_INCH => 1,
        img_pdn::PDU_CM => 2,
        _ => 0,
    };
    let d = p.doc.dm(doc);
    if u != 0 {
        d.dpu_unit = u;
        d.dpu_x = dpux;
        d.dpu_y = dpuy;
    }
    d.cur = d.layers.len() as i32 - 1;
    crate::document::model::doc_mark_layers(p, doc);
    if path.is_empty() {
        // recovered after a session end: unsaved, no file yet (fio_recover gives it one)
        crate::document::history::hist_init_unsaved(p, doc, "Wiederhergestellt", IC_FOLDER_OPEN);
        doc_set_active(p, doc);
    } else {
        opened(p, ui, doc, path, IMGF_PDN);
    }
    0
}

fn install_psd(p: &mut Pn, ui: &mut Ui, psd: img_all::psd::document::Document, path: &str) -> i32 {
    let Some(doc) = crate::document::model::doc_new(p, psd.meta.w as i32, psd.meta.h as i32, 0, DNF_EMPTY as u32) else { return ERR_NOMEM; };
    for (index, layer) in psd.layers.into_iter().enumerate() {
        let i = match crate::document::layers::lay_add_sf(p, doc, index as i32, layer.pixels, Some(&layer.name)) {
            Ok(i) => i,
            Err(_) => { crate::document::model::doc_close(p, doc); return ERR_NOMEM; },
        };
        let l = &mut p.doc.dm(doc).layers[i as usize];
        l.sf.opac = layer.opacity;
        l.sf.blend = layer.blend;
        l.flags = if layer.visible { LYF_VISIBLE as u32 } else { 0 };
    }
    let d = p.doc.dm(doc);
    set_dpi(d, psd.meta.dpix, psd.meta.dpiy, psd.meta.resunit);
    d.icc = psd.meta.icc.clone();
    d.meta = Some(alloc::boxed::Box::new(psd.meta));
    d.cur = d.layers.len() as i32 - 1;
    crate::document::model::doc_mark_layers(p, doc);
    if psd.converted {
        crate::document::history::hist_init_unsaved(p, doc, "PSD-Ebenen importieren", IC_FOLDER_OPEN);
        fio_error(p, Some("PSD-Import"), "Die Rasterebenen wurden als neues 8-Bit-Dokument importiert. Masken wurden angewendet; höhere Farbtiefen, Bereiche außerhalb der Leinwand oder zusätzliche Photoshop-Daten bleiben in der Originaldatei. Bitte unter einem neuen Namen speichern.");
    } else {
        opened(p, ui, doc, path, img_core::IMGF_PSD);
    }
    0
}

// ------------------------------------------------------------------ session end / recovery
//
// A session end (Windows log-off or shutdown, SIGTERM) leaves no time for "Änderungen
// speichern?".  Every modified image is then written to the recovery folder as <run>-<n>.pdn (all
// layers) with <run>-<n>.ini (the file it came from); the next start opens them again, still
// modified, and removes the copies.  A copy only counts as complete under its .pdn name: it is
// written as .tmp and renamed last.

// recovery_dir(ui) -> --config DIR or $PaintCraft_CONFIG_DIR + "/recovery" (like the settings),
// a headless run without them keeps it in memory, else the app's data folder
fn recovery_dir(ui: &Ui) -> String {
    let dir = plat::arg_value("--config")
        .filter(|v| !v.is_empty())
        .or_else(|| plat::env("PaintCraft_CONFIG_DIR").filter(|v| !v.is_empty()));
    match dir {
        Some(d) if d.len() < PATH_CAP - 64 => format!("{}/recovery", d.trim_end_matches(['/', '\\'])),
        _ if ui.headless() => String::from("mem://paintcraft/recovery"),
        _ => String::from("data://recovery"),
    }
}

/// fio_session_end() -- UIC_SESSION_END: a recovery copy of every modified image
pub(crate) fn fio_session_end(p: &mut Pn, ui: &Ui) {
    let docs: Vec<DocId> = p.doc.docs.iter().filter(|d| d.flags & DF_MODIFIED as u32 != 0).map(|d| d.id).collect();
    if docs.is_empty() {
        return;
    }
    let dir = recovery_dir(ui);
    plat::mkdir(&dir);
    let run = format!("{:x}", plat::unix_time_ns());
    for (n, doc) in docs.into_iter().enumerate() {
        let base = format!("{}/{}-{}", dir, run, n);
        let (tmp, ini_path, pdn) = (format!("{}.tmp", base), format!("{}.ini", base), format!("{}.pdn", base));
        let mut ini = Ini::new();
        {
            let d = p.doc.d(doc);
            if let Some(path) = d.path.as_deref() {
                ini.set("recovery", "path", path);
                ini.set_int("recovery", "fmt", d.fmt as i64);
            }
        }
        let ok = save_pdn(p, doc, &tmp) == 0 && ini.save(&ini_path) && plat::rename(&tmp, &pdn);
        if !ok {
            plat::delete(&tmp);
            plat::delete(&ini_path);
        }
    }
}

// absolute_path(s) -- a drive path, a UNC path or a rooted path of the operating system
fn absolute_path(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() < PATH_CAP
        && !s.contains('\0')
        && (matches!(b.first(), Some(b'/') | Some(b'\\')) || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':'))
}

// fio_recover() -- the copies of the last session end are open again (modified, "Wiederhergestellt");
// a copy that cannot be read is removed, one that does not fit (DOC_MAX, memory) stays for the next
// start; leftovers of an interrupted write go
fn fio_recover(p: &mut Pn, ui: &mut Ui) {
    let dir = recovery_dir(ui);
    let (mut copies, mut other): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    plat::dir_list(&dir, &mut |name, is_dir, _| {
        if is_dir {
            return;
        }
        if name.ends_with(".pdn") {
            copies.push(String::from(name));
        } else if name.ends_with(".tmp") || name.ends_with(".ini") {
            other.push(String::from(name));
        }
    });
    for name in other {
        let has_copy = name.strip_suffix(".ini").is_some_and(|b| copies.contains(&format!("{}.pdn", b)));
        if !has_copy {
            plat::delete(&format!("{}/{}", dir, name));
        }
    }
    copies.sort();
    for name in copies {
        if doc_count(&p.doc) >= DOC_MAX {
            break;
        }
        let file = format!("{}/{}", dir, name);
        let ini_path = format!("{}.ini", file.trim_end_matches(".pdn"));
        let Some(data) = plat::read_file(&file) else { continue };
        match decode_open(&data, &file, None) {
            Ok(LoadedImage::Pdn(pd)) => {
                if install_pdn(p, ui, pd, "") != 0 {
                    continue;
                }
                recovered_origin(p, &ini_path);
            }
            Err(ERR_NOMEM) => continue,
            _ => {}
        }
        plat::delete(&file);
        plat::delete(&ini_path);
    }
}

// recovered_origin(ini_path) -- the file the recovered (active) image belonged to, when it is
// still a valid absolute path of its own: Speichern then writes there again
fn recovered_origin(p: &mut Pn, ini_path: &str) {
    let Some(doc) = doc_active(&p.doc) else { return };
    let ini = Ini::load(ini_path);
    let Some(origin) = ini.get("recovery", "path").filter(|v| absolute_path(v)) else { return };
    let origin = canon_path(origin);
    if doc_find_path(&p.doc, &origin).is_some() {
        return; // that file is open in another tab
    }
    let fmt = ini.get_int("recovery", "fmt", 0);
    crate::document::model::doc_set_path(p, doc, &origin);
    if fmt > 0 && fmt < IMGF_COUNT as i64 {
        let d = p.doc.dm(doc);
        d.fmt = fmt as u32;
        d.saveopt = img_core::io::opt_init(fmt as u32);
    }
}

// ------------------------------------------------------------------ recent files

// recent_add(path) -- to the top of the list (an existing entry moves up), at most RECENT_MAX
fn recent_add(p: &mut Pn, _ui: &mut Ui, path: &str) {
    recent_add_p(p, path);
}

fn recent_add_p(p: &mut Pn, path: &str) {
    let f = &mut p.fio;
    let path = cap(String::from(path), PATH_CAP);
    if let Some(k) = f.recent.iter().position(|r| *r == path) {
        f.recent.remove(k);
    } else if f.recent.len() >= RECENT_MAX as usize {
        f.recent.truncate(RECENT_MAX as usize - 1);
    }
    f.recent.insert(0, path);
    recent_labels(p);
}

// recent_labels() -- menu labels "&1 name" (the file name, '&' doubled)
fn recent_labels(p: &mut Pn) {
    for i in 0..p.fio.recent.len() {
        let mut l: Vec<u8> = if i < 9 { format!("&{} ", i + 1).into_bytes() } else { Vec::from(&b"1&0 "[..]) };
        for &c in file_part(&p.fio.recent[i]).as_bytes() {
            if l.len() >= LABEL_CAP - 3 {
                break;
            }
            l.push(c);
            if c == b'&' {
                l.push(c);
            }
        }
        let s = String::from_utf8_lossy(&l).into_owned();
        pn_cmd_label(p, CMD_RECENT0 + i as u32, &s);
    }
}

// ------------------------------------------------------------------ save

// save_pdn(doc, path) -> 0 / ERR_* -- all layers through pdn_encode
fn save_pdn(p: &Pn, doc: DocId, path: &str) -> i32 {
    let d = p.doc.d(doc);
    let unit = match d.dpu_unit {
        1 => img_pdn::PDU_INCH,
        2 => img_pdn::PDU_CM,
        _ => img_pdn::PDU_PIXEL,
    };
    let layers = d
        .layers
        .iter()
        .map(|l| img_pdn::LayerRef {
            sf: &l.sf,
            name: l.name.as_bytes(),
            opacity: l.sf.opac,
            visible: (l.flags & LYF_VISIBLE as u32) as u8,
            blend: l.sf.blend,
            isbg: (l.flags >> 1 & 1) as u8,
            meta: &l.meta,
        })
        .collect();
    let dr = img_pdn::DocRef { w: d.w as u32, h: d.h as u32, unit, dpux: d.dpu_x, dpuy: d.dpu_y, layers, nvc: &[], icc: &[] };
    let opt = ImgOpt { fmt: IMGF_PDN, ..ImgOpt::default() };
    match img_pdn::encode(&dr, Some(&opt)) {
        Ok(b) => {
            if plat::write_file(path, &b) {
                0
            } else {
                ERR_IO
            }
        }
        Err(e) => e,
    }
}

// save_flat(doc, path, fmt) -> 0 / ERR_* -- the composite with the document's options (ICC /
// EXIF / XMP embedded when the document has them)
fn save_flat(p: &mut Pn, doc: DocId, path: &str, fmt: u32) -> i32 {
    {
        let d = p.doc.dm(doc);
        if d.saveopt.fmt != fmt {
            d.saveopt = img_core::io::opt_init(fmt);
        }
    }
    crate::document::composite::comp_update(p, doc);
    let d = p.doc.d(doc);
    let mut opt: ImgOpt = d.saveopt;
    opt.fmt = fmt;
    let mut meta = ImgMeta::default();
    let use_meta = fio_prep_opt(d, &mut opt, &mut meta);
    if use_meta {
        opt.meta = Some(&meta);
    }
    let sf = crate::document::composite::comp_sf(d);
    if sf.px.is_empty() {
        return ERR_NOMEM;
    }
    img_core::io::save_opt(path, sf, Some(&opt))
}

/// fio_prep_opt(doc, opt, meta) -- meta = the document's metadata (EXIF / XMP / ICC copied in);
/// opt gets IOF_EMBED_ICC / EXIF / XMP for the blobs it has; -> true when opt.meta should
/// point to meta (the caller sets it: the borrow of meta)
pub(crate) fn fio_prep_opt(d: &Doc, opt: &mut ImgOpt, meta: &mut ImgMeta) -> bool {
    opt.flags &= !(IOF_EMBED_ICC | IOF_EMBED_EXIF | IOF_EMBED_XMP);
    opt.meta = None;
    *meta = match &d.meta {
        Some(m) => (**m).clone(),
        None => ImgMeta::default(),
    };
    meta.icc = d.icc.clone();
    let mut f = 0;
    if !meta.icc.is_empty() {
        f |= IOF_EMBED_ICC;
    }
    if !meta.exif.is_empty() {
        f |= IOF_EMBED_EXIF;
    }
    if !meta.xmp.is_empty() {
        f |= IOF_EMBED_XMP;
    }
    let scale=if d.dpu_unit==2 {2.54}else{1.0};
    meta.dpix=(d.dpu_x*scale*65536.0).clamp(1.0,u32::MAX as f64) as u32;
    meta.dpiy=(d.dpu_y*scale*65536.0).clamp(1.0,u32::MAX as f64) as u32;
    meta.resunit=if d.dpu_unit==2 {IMR_CM}else{img_core::IMR_INCH};
    opt.flags |= f;
    true
}

// do_save -> 0 saved, 1 awaiting browser storage, or a negative error.
fn do_save(p: &mut Pn, ui: &mut Ui, doc: DocId, path: &str, fmt: u32) -> i32 {
    let r = if fmt == IMGF_PDN { save_pdn(p, doc, path) }
        else if fmt == img_core::IMGF_PSD { save_psd(p, doc, path) }
        else { save_flat(p, doc, path, fmt) };
    save_result(p, ui, doc, path, fmt, r)
}

fn save_psd(p: &mut Pn, doc: DocId, path: &str) -> i32 {
    crate::document::composite::comp_update(p, doc);
    let d = p.doc.d(doc);
    let mut meta = ImgMeta::default();
    let mut opt = ImgOpt { fmt: img_core::IMGF_PSD, ..ImgOpt::default() };
    fio_prep_opt(d, &mut opt, &mut meta);
    opt.meta = Some(&meta);
    let layers: Vec<_> = d.layers.iter().map(|l| img_all::psd::document::LayerRef {
        pixels: &l.sf, name: &l.name, visible: l.flags & LYF_VISIBLE as u32 != 0,
        opacity: l.sf.opac, blend: l.sf.blend,
    }).collect();
    match img_all::psd::document::encode(crate::document::composite::comp_sf(d), &layers, &opt) {
        Ok(data) => if plat::write_file(path, &data) { 0 } else { ERR_IO },
        Err(e) => e,
    }
}

// Native writes complete immediately; browser writes still need durable storage.
fn save_result(p: &mut Pn, ui: &mut Ui, doc: DocId, path: &str, fmt: u32, err: i32) -> i32 {
    #[cfg(target_arch = "wasm32")]
    if err == 0 {
        if let Some(request) = plat::web::persist(path) {
            p.fio.pending_save = Some(PendingSave { request, doc, path: String::from(path), fmt });
            ui.repass();
            return 1; // the close/save-all chain must wait for durable storage
        }
        return finish_save(p, ui, doc, path, fmt, ERR_IO);
    }
    finish_save(p, ui, doc, path, fmt, err)
}

fn finish_save(p: &mut Pn, ui: &mut Ui, doc: DocId, path: &str, fmt: u32, err: i32) -> i32 {
    if err == 0 {
        recent_add(p, ui, path);
        set_last_dir(&mut p.fio, path);
        crate::document::model::doc_set_path(p, doc, path);
        p.doc.dm(doc).fmt = fmt;
        doc_set_modified(p, doc, false);
        crate::document::history::hist_mark_clean(p, doc);
        p.fio.err_msg.clear();
        return 0;
    }
    let msg = match err {
        ERR_IO => "Die Datei ließ sich nicht auf den Datenträger schreiben.",
        ERR_NOMEM => "Zu wenig Arbeitsspeicher, um das Bild zu speichern.",
        ERR_UNSUPPORTED => "In diesem Format kann PaintCraft nicht speichern.",
        _ => "Das Speichern ist aus unbekanntem Grund fehlgeschlagen.",
    };
    let t = String::from(file_part(path));
    fio_error(p, Some(&t), msg);
    err
}

pub(crate) fn fio_save_busy(p: &Pn) -> bool {
    #[cfg(target_arch = "wasm32")]
    { p.fio.pending_save.is_some() }
    #[cfg(not(target_arch = "wasm32"))]
    { let _ = p; false }
}

#[cfg(target_arch = "wasm32")]
fn poll_save(p: &mut Pn, ui: &mut Ui) {
    use plat::web::RequestStatus;
    let Some(save) = &p.fio.pending_save else { return };
    let status = plat::web::request_status(save.request);
    if status == Some(RequestStatus::Pending) { return; }
    let Some(save) = p.fio.pending_save.take() else { return };
    plat::web::forget_request(save.request);
    if p.doc.get(save.doc).is_none() {
        chain_abort(&mut p.fio);
        ui.repass();
        return;
    }
    let err = if status == Some(RequestStatus::Durable) { 0 } else { ERR_IO };
    let result = finish_save(p, ui, save.doc, &save.path, save.fmt, err);
    after(p, ui, result);
    ui.repass();
}

// save_options_done -> 0 saved / 1 pending / negative failed or cancelled. The save options dialog
// closed: OK (DLG_PRIMARY) writes the bytes it encoded to sv_path
fn save_options_done(p: &mut Pn, ui: &mut Ui, result: u32) -> i32 {
    let doc = p.fio.sv_doc;
    let Some(doc) = doc.filter(|&d| result == DLG_PRIMARY && doc_index(&p.doc, d) >= 0) else {
        crate::workspace::dialogs::save_options::saveopt_close(p, ui);
        return -1;
    };
    let err = match crate::workspace::dialogs::save_options::saveopt_take(p, ui) {
        Ok(b) => {
            if plat::write_file(&p.fio.sv_path, &b) {
                crate::workspace::dialogs::save_options::saveopt_accept(p);
                0
            } else {
                ERR_IO
            }
        }
        Err(e) => e,
    };
    crate::workspace::dialogs::save_options::saveopt_close(p, ui);
    let path = p.fio.sv_path.clone();
    let fmt = p.fio.sv_fmt;
    save_result(p, ui, doc, &path, fmt, err)
}

// request_save(doc, path, fmt) -> 0 saved / 1 waiting for a question / -1 failed or
// cancelled.  Layered images in a flat format ask "Speichern" (Zusammenfassen) first.
fn request_save(p: &mut Pn, ui: &mut Ui, doc: DocId, path: Option<&str>, fmt: u32) -> i32 {
    if let Some(path) = path {
        p.fio.sv_path = canon_path(path);
        if p.fio.sv_path.is_empty() {
            fio_error(p, None, "Der Pfad ist zu lang.");
            p.fio.sv_saveas = false;
            return -1;
        }
    }
    p.fio.sv_doc = Some(doc);
    p.fio.sv_fmt = fmt;
    if !encodable(fmt) {
        let t = String::from(file_part(&p.fio.sv_path));
        fio_error(p, Some(&t), "In diesem Format kann PaintCraft nicht speichern.");
        p.fio.sv_saveas = false;
        return -1;
    }
    if fmt != IMGF_PDN && fmt != img_core::IMGF_PSD && p.doc.d(doc).layers.len() > 1 {
        p.fio.flat_open.set(1);
        p.fio.flat_pending = true;
        ui.repass();
        return 1;
    }
    save_now(p, ui)
}

// save_now() -> 0 saved / 1 waiting for the save options dialog / -1 failed -- sv_doc to
// sv_path as sv_fmt (after the questions)
fn save_now(p: &mut Pn, ui: &mut Ui) -> i32 {
    let Some(doc) = p.fio.sv_doc else { return -1 };
    let fmt = p.fio.sv_fmt;
    let saveas = core::mem::take(&mut p.fio.sv_saveas);
    if crate::workspace::dialogs::save_options::saveopt_needed(p, doc, fmt, saveas) {
        if !crate::workspace::dialogs::save_options::saveopt_open(p, ui, doc, fmt) {
            return -1;
        }
        return 1;
    }
    let path = p.fio.sv_path.clone();
    do_save(p, ui, doc, &path, fmt)
}

// start_save_as(doc) -- the save dialog for doc: its path without the extension (or its
// name), the type preselected (.pdn for layered images when possible, else the file's own
// format, else PNG)
fn start_save_as(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    p.fio.sa_doc = Some(doc);
    let d = p.doc.d(doc);
    match &d.path {
        None => {
            let name = d.name.clone();
            dir_path(&mut p.fio, &name);
        }
        Some(path) => {
            let mut s = cap(path.clone(), PATH_CAP);
            // strip the extension of the file name part
            let fp = s.len() - file_part(&s).len();
            if let Some(k) = s[fp..].rfind('.') {
                s.truncate(fp + k);
            }
            p.fio.fd_path = s;
        }
    }
    let d = p.doc.d(doc);
    let mut fmt = IMGF_PNG;
    if encodable(d.fmt) {
        fmt = d.fmt;
    }
    if d.layers.len() > 1 && encodable(IMGF_PDN) {
        fmt = IMGF_PDN;
    }
    let k = p.fio.save_fmts.iter().position(|&f| f == fmt).unwrap_or(0) as u32;
    p.fio.fds_flags = k << FD_EXT_SHIFT | FD_SAVE;
    p.fio.fds_open.set(1);
    p.fio.fds_pending = true;
    ui.repass();
}

// save_for_chain(doc) -> 0 saved / 1 waiting / -1 failed: to its file when possible, else
// through the save dialog
fn save_for_chain(p: &mut Pn, ui: &mut Ui, doc: DocId) -> i32 {
    let d = p.doc.d(doc);
    if let Some(path) = d.path.clone() {
        if encodable(d.fmt) {
            let fmt = d.fmt;
            return request_save(p, ui, doc, Some(&path), fmt);
        }
    }
    start_save_as(p, ui, doc);
    1
}

// ------------------------------------------------------------------ question chains

// ask(doc) -- "Änderungen speichern?" for doc (made the active tab)
fn ask(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    p.fio.ask_doc = Some(doc);
    doc_set_active(p, doc);
    p.fio.ask_open.set(1);
    p.fio.ask_pending = true;
    ui.repass();
}

// first_modified() -> the first open document with unsaved changes
fn first_modified(p: &Pn) -> Option<DocId> {
    p.doc.docs.iter().find(|d| d.flags & DF_MODIFIED as u32 != 0).map(|d| d.id)
}

// chain_abort()
fn chain_abort(f: &mut Fio) {
    f.chain = CH_NONE;
    f.chain_doc = None;
}

// chain_step() -- chain_doc is saved or its changes discarded: go on
fn chain_step(p: &mut Pn, ui: &mut Ui) {
    match p.fio.chain {
        CH_CLOSE => {
            let d = p.fio.chain_doc;
            close_now(p, d);
            chain_abort(&mut p.fio);
        }
        CH_EXIT => {
            if let Some(d) = p.fio.chain_doc {
                close_now(p, Some(d));
                p.fio.chain_doc = None;
            }
            match first_modified(p) {
                Some(d) => {
                    p.fio.chain_doc = Some(d);
                    ask(p, ui, d);
                }
                None => {
                    chain_abort(&mut p.fio);
                    ui.quit();
                }
            }
        }
        CH_SAVEALL => loop {
            let Some(d) = first_modified(p) else {
                chain_abort(&mut p.fio);
                return;
            };
            p.fio.chain_doc = Some(d);
            doc_set_active(p, d);
            let r = save_for_chain(p, ui, d);
            if r == 0 {
                continue; // saved: the next one
            }
            if r < 0 {
                chain_abort(&mut p.fio);
            }
            return; // waiting for a dialog
        },
        _ => {}
    }
}

// close_now(doc) -- close without asking (still open?)
fn close_now(p: &mut Pn, doc: Option<DocId>) {
    if let Some(d) = doc {
        if doc_index(&p.doc, d) >= 0 {
            crate::document::model::doc_close(p, d);
        }
    }
}

/// fio_close_doc(doc) -- Schließen with the question for unsaved changes
pub(crate) fn fio_close_doc(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    if p.fio.chain != CH_NONE || fio_save_busy(p) {
        return; // a question is open already
    }
    let Some(d) = p.doc.get(doc) else { return };
    if d.flags & DF_MODIFIED as u32 == 0 {
        crate::document::model::doc_close(p, doc);
        return;
    }
    p.fio.chain = CH_CLOSE;
    p.fio.chain_doc = Some(doc);
    ask(p, ui, doc);
}

// exit_request() -> true = the window stays (questions follow), false = nothing to ask
fn exit_request(p: &mut Pn, ui: &mut Ui) -> bool {
    if p.fio.chain != CH_NONE {
        return true;
    }
    if first_modified(p).is_none() {
        return false;
    }
    p.fio.chain = CH_EXIT;
    p.fio.chain_doc = None;
    chain_step(p, ui);
    true
}

// ------------------------------------------------------------------ module

/// fio_states() -- Datei commands, recent rows, window title (the first call opens the
/// command line files)
pub(crate) fn fio_states(p: &mut Pn, ui: &mut Ui) {
    #[cfg(target_arch = "wasm32")]
    poll_save(p, ui);
    if !p.fio.started {
        p.fio.started = true;
        fio_start(p, ui);
    }
    let n = doc_count(&p.doc);
    pn_cmd_set(ui, CMD_NEW, 0);
    pn_cmd_set(ui, CMD_OPEN, 0);
    let f = if n != 0 { 0 } else { CF_DISABLED };
    pn_cmd_set(ui, CMD_SAVE, f);
    pn_cmd_set(ui, CMD_SAVE_AS, f);
    pn_cmd_set(ui, CMD_SAVE_ALL, f);
    pn_cmd_set(ui, CMD_CLOSE, f);
    // recent files
    let nr = p.fio.recent.len();
    for i in 0..RECENT_MAX as usize {
        pn_cmd_set(ui, CMD_RECENT0 + i as u32, if i < nr { 0 } else { CF_HIDDEN });
    }
    pn_cmd_set(ui, CMD_RECENT_NONE, if nr == 0 { CF_DISABLED } else { CF_HIDDEN });
    pn_cmd_set(ui, CMD_RECENT_CLEAR, if nr != 0 { 0 } else { CF_DISABLED });
    update_title(p, ui);
    crate::integrations::printing::prt_states(p, ui); // ---- PR1 (print.rs): Drucken… state
}

// update_title() -- "Name – PaintCraft" ('*' in front while modified), "PaintCraft" without images
fn update_title(p: &mut Pn, ui: &mut Ui) {
    #[cfg(target_arch = "wasm32")]
    {
        let u = p.doc.docs.iter().any(|d| d.flags & DF_MODIFIED as u32 != 0);
        if u != p.fio.unsaved {
            p.fio.unsaved = u;
            plat::set_unsaved(u);
        }
    }
    let t = match p.doc.active() {
        None => String::from("PaintCraft"),
        Some(d) => {
            if d.flags & DF_MODIFIED as u32 != 0 {
                format!("*{} – PaintCraft", d.name)
            } else {
                format!("{} – PaintCraft", d.name)
            }
        }
    };
    let t = cap(t, TITLE_CAP);
    if t == p.fio.title {
        return;
    }
    p.fio.title = t;
    ui.set_title(&p.fio.title);
}

/// fio_frame() -- file dialogs, questions, the "Neu" dialog
pub(crate) fn fio_frame(p: &mut Pn, ui: &mut Ui) {
    open_window(p, ui);
    // open
    let f = &mut p.fio;
    let open = f.fdo_open.clone();
    let exts: Vec<&str> = f.open_exts.iter().map(|s| s.as_str()).collect();
    if ui.filedlg(ID_FD_OPEN, FD_OPEN, &open, &mut f.fd_path, PATH_CAP as u32, &exts) & EV_ACT != 0 {
        ui.cmd_run(CMD_FIO_OPENED, 0);
    }
    let open = f.fds_open.clone();
    let exts: Vec<&str> = f.save_exts.iter().map(|s| s.as_str()).collect();
    if ui.filedlg(ID_FD_SAVE, f.fds_flags, &open, &mut f.fd_path, PATH_CAP as u32, &exts) & EV_ACT != 0 {
        f.fds_pending = false;
        ui.cmd_run(CMD_FIO_SAVED, 0);
    } else if f.fds_pending && f.fds_open.get() == 0 {
        f.fds_pending = false;
        ui.cmd_run(CMD_FIO_FDCANCEL, 0);
    }
    ask_window(p, ui);
    flat_window(p, ui);
    crate::workspace::dialogs::new_image::dlgnew_frame(p, ui);
    crate::workspace::dialogs::save_options::saveopt_frame(p, ui);
    crate::integrations::printing::prt_frame(p, ui); // ---- PR1 (print.rs): the print dialog
    #[cfg(target_arch = "wasm32")]
    if fio_save_busy(p) {
        if ui.window_begin(WINID_SAVE_PENDING, crate::preferences::localization::tr("Speichern"), None,
            WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT | WF_NOCLOSE, None) {
            ui.text(TS_BODY, crate::preferences::localization::tr("Das Bild wird im Browserspeicher gesichert …"));
        }
        ui.window_end();
    }
}

// ask_window() -- "Änderungen speichern?" (modal dialog window)
fn ask_window(p: &mut Pn, ui: &mut Ui) {
    let f = &mut p.fio;
    if f.ask_pending && f.ask_open.get() == 0 {
        // closed with Esc / the close button: like Abbrechen
        f.ask_pending = false;
        ui.cmd_run(CMD_FIO_ASKED, DLG_CLOSE);
    }
    let open = f.ask_open.clone();
    let rect = f.ask_rect.clone();
    if ui.window_begin(WINID_ASK, crate::preferences::localization::tr("Änderungen speichern?"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let doc = p.fio.ask_doc;
        if doc_count(&p.doc) != 0 {
            let d = doc.and_then(|id| p.doc.get(id));
            ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
            if let Some(d) = d {
                if !d.thumb.px.is_empty() {
                    let sf = ask_thumb(&mut p.fio, d);
                    ui.image(0, &sf, d.thumb_aux);
                    // a hairline around the thumbnail (white images on the white dialog)
                    let c = ui.color(T_CTRL_STROKE);
                    ui.draw_color(c);
                    let hair = ui.hair();
                    let r = ui.last_rect();
                    ui.draw_frame(r[0], r[1], r[2], r[3], 0, hair);
                }
            }
            let name = d.map_or(String::new(), |d| d.name.clone());
            let t = cap(format!("„{}“ wurde geändert und noch nicht gespeichert. Wie möchten Sie fortfahren?", name), ERR_CAP);
            ui.next_size(300, 0);
            ui.text(TS_BODY | TX_WRAP, &t);
            ui.box_end();
        }
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("Speichern")), Some(crate::preferences::localization::tr("Nicht speichern")), Some(crate::preferences::localization::tr("Abbrechen")));
        if r != 0 {
            p.fio.ask_pending = false;
            ui.cmd_run(CMD_FIO_ASKED, r);
        }
    }
    ui.window_end();
}

// ask_thumb(doc) -- the renderer's copy of the document's thumbnail (renewed when it changed)
fn ask_thumb(f: &mut Fio, d: &Doc) -> Rc<Surface> {
    if let Some((id, aux, sf)) = &f.thumb {
        if *id == d.id && *aux == d.thumb_aux {
            return sf.clone();
        }
    }
    let s = &d.thumb;
    let sf = Rc::new(Surface { px: s.px.clone(), w: s.w, h: s.h, stride: s.stride, fmt: s.fmt, flags: s.flags, opac: s.opac, blend: s.blend });
    f.thumb = Some((d.id, d.thumb_aux, sf.clone()));
    sf
}

// flat_window() -- "Speichern": flatten before saving in a format without layers
fn flat_window(p: &mut Pn, ui: &mut Ui) {
    let f = &mut p.fio;
    if f.flat_pending && f.flat_open.get() == 0 {
        f.flat_pending = false;
        ui.cmd_run(CMD_FIO_FLAT, DLG_CLOSE);
    }
    let open = f.flat_open.clone();
    let rect = f.flat_rect.clone();
    if ui.window_begin(WINID_FLAT, crate::preferences::localization::tr("Speichern"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        ui.text(
            TS_BODY | TX_WRAP,
            "Dieses Dateiformat kennt keine Ebenen. Für die Datei werden deshalb alle Ebenen zu einer einzigen vereint.",
        );
        ui.text(
            TS_CAPTION | TX_WRAP | TX_SECONDARY,
            "Das betrifft nur die gespeicherte Datei; im geöffneten Dokument bleiben alle Ebenen erhalten.",
        );
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("Zusammenfassen")), None, Some(crate::preferences::localization::tr("Abbrechen")));
        if r != 0 {
            p.fio.flat_pending = false;
            ui.cmd_run(CMD_FIO_FLAT, r);
        }
    }
    ui.window_end();
}

// after(result) -- a save of the chain ended: 0 saved -> next step, < 0 -> abort
fn after(p: &mut Pn, ui: &mut Ui, r: i32) {
    if r > 0 {
        return;
    }
    if r < 0 {
        chain_abort(&mut p.fio);
        return;
    }
    chain_step(p, ui);
}

/// fio_cmd(id, arg) -> 1 handled (UIC_CLOSE: 1 = keep the window)
pub(crate) fn fio_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    // ---- PR1 (print.rs): Drucken… and its dialog
    if id == CMD_PRINT || id == CMD_PRT_DONE {
        return crate::integrations::printing::prt_cmd(p, ui, id, arg);
    }
    match id {
        UIC_CLOSE => return exit_request(p, ui),
        UIC_OPEN => {
            // AF_SINGLE_INSTANCE: a later start of PaintCraft hands its files over
            for r in ui.open_take() {
                fio_open_args(p, ui, &r.cwd, &r.args);
            }
            return true;
        }
        UIC_WAKE => {
            poll_open(p, ui);
            #[cfg(target_arch = "wasm32")]
            poll_save(p, ui);
            crate::workspace::dialogs::save_options::saveopt_poll(p, ui);
            return false;
        }
        CMD_EXIT => {
            if !exit_request(p, ui) {
                ui.quit();
            }
        }
        CMD_NEW => crate::workspace::dialogs::new_image::dlgnew_open(p, ui),
        CMD_FIO_NEWDO => {
            crate::workspace::dialogs::new_image::dlgnew_create(p, ui);
        }
        CMD_OPEN => {
            // start at the active image's file, else in the folder used last
            dir_path(&mut p.fio, "");
            if let Some(d) = p.doc.active() {
                if let Some(path) = &d.path {
                    p.fio.fd_path = cap(path.clone(), PATH_CAP);
                }
            }
            p.fio.fdo_open.set(1);
        }
        CMD_FIO_OPENED => {
            let path = p.fio.fd_path.clone();
            fio_queue_path(p, ui, &path);
        }
        CMD_SAVE => {
            if let Some(d) = doc_active(&p.doc) {
                if p.fio.chain == CH_NONE {
                    save_for_chain(p, ui, d);
                }
            }
        }
        CMD_SAVE_AS => {
            if let Some(d) = doc_active(&p.doc) {
                if p.fio.chain == CH_NONE {
                    start_save_as(p, ui, d);
                }
            }
        }
        CMD_FIO_SAVED => match p.fio.sa_doc.filter(|&d| doc_index(&p.doc, d) >= 0) {
            Some(d) => {
                let path = p.fio.fd_path.clone();
                let fmt = img_core::io::format_from_path(&path);
                p.fio.sv_saveas = true;
                let r = request_save(p, ui, d, Some(&path), fmt);
                after(p, ui, r);
            }
            None => chain_abort(&mut p.fio),
        },
        CMD_FIO_FDCANCEL => chain_abort(&mut p.fio),
        CMD_FIO_SOPT => {
            let r = save_options_done(p, ui, arg);
            after(p, ui, r);
        }
        CMD_FIO_FLAT => {
            if arg == DLG_PRIMARY {
                let r = save_now(p, ui);
                after(p, ui, r);
            } else {
                p.fio.sv_saveas = false;
                chain_abort(&mut p.fio);
            }
        }
        CMD_FIO_ASKED => {
            if arg == DLG_PRIMARY {
                let r = match p.fio.ask_doc {
                    Some(d) => save_for_chain(p, ui, d),
                    None => -1,
                };
                after(p, ui, r);
            } else if arg == DLG_SECONDARY {
                if let Some(d) = p.fio.ask_doc {
                    doc_set_modified(p, d, false);
                }
                chain_step(p, ui);
            } else {
                chain_abort(&mut p.fio);
            }
        }
        CMD_SAVE_ALL => {
            if p.fio.chain == CH_NONE {
                p.fio.chain = CH_SAVEALL;
                chain_step(p, ui);
            }
        }
        CMD_CLOSE => {
            if let Some(d) = doc_active(&p.doc) {
                fio_close_doc(p, ui, d);
            }
        }
        CMD_RECENT_CLEAR => p.fio.recent.clear(),
        _ => {
            let k = id.wrapping_sub(CMD_RECENT0);
            if k >= RECENT_MAX as u32 {
                return false;
            }
            if (k as usize) < p.fio.recent.len() {
                let path = p.fio.recent[k as usize].clone(); // a copy: opening reorders the list
                fio_queue_path(p, ui, &path);
            }
        }
    }
    true
}

/// fio_recent_count() -> entries of "Zuletzt geöffnet"
pub(crate) fn fio_recent_count(f: &Fio) -> i32 {
    f.recent.len() as i32
}

/// fio_recent_path(i) -> path of entry i (0 = the newest) | None
pub(crate) fn fio_recent_path(f: &Fio, i: i32) -> Option<&str> {
    f.recent.get(i as u32 as usize).map(|s| s.as_str())
}

/// fio_recent_add(path) -- put a path on top of the list (settings.rs adds the saved list
/// oldest first); empty paths and paths of PATH_CAP bytes or more are ignored
pub(crate) fn fio_recent_add(p: &mut Pn, path: &str) {
    if path.is_empty() || path.len() >= PATH_CAP {
        return;
    }
    recent_add_p(p, path);
}
