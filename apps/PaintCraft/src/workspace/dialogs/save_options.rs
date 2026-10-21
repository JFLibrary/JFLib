// dlg_saveopt -- PaintCraft: "Speicheroptionen" (paint.net's Save
// Configuration dialog).  Left: "Einstellungen" with exactly the options the format's encoder
// honours (IFI_IOM: Bit-Tiefe, Dithering-Level, Schwelle für Transparenz, Qualität,
// Verlustfrei, Chroma-Subsampling, Preset, Bemühung / Tempo der Kompression, RLE,
// Komprimierung, DDS format / Fehlermetrik / Cube-Karte / Mip Maps, Zeilensprung) and
// "Standardwerte"; right: "Vorschau – Dateigröße …" over the image as it will be saved: a
// worker encodes the composite with the options (metadata included, so the size is exact)
// and decodes the bytes again; an option change cancels and restarts it.  "Ganzes Bild" /
// "Originalgröße" (click on the preview toggles, drag pans).  OK hands the encoded bytes to
// fileio.rs (CMD_FIO_SOPT), so the file is exactly what the preview showed.
//                                              [apps/PaintCraft/src/dlg_saveopt.s]
//
// Sub-module of fileio.rs:
//   saveopt_needed(doc, fmt, saveas) -> true = ask first: Speichern unter, or the first save
//                                 of the document in this format (paint.net); never .pdn or a
//                                 format without options
//   saveopt_open(doc, fmt)        the dialog with the document's options for fmt
//   saveopt_frame()               the window (fio_frame)
//   saveopt_poll()                UI thread: collect / restart the preview job (frame, UIC_WAKE)
//   saveopt_pending() -> bool     the preview is computed (script `sync`; tool::SYNC_SAVEOPT)
//   saveopt_take() -> bytes | ERR_*   OK: the file bytes; the options become the document's
//                                 (DOC_SAVEOPT, DF_OPTOK)
//   saveopt_close()               cancel the job, free the preview
// Widget ids ID_DLG_SAVEOPT + n, window WINID_DLG0 + 3.  Probes: ID_SO_PROBE value = file size
// (-1 while computing / on errors), text "busy" / "ok" / "err" / "none" (no preview);
// ID_SO_PROBE + 1 value = size of the result before, text "kleiner" / "größer" / "gleich" /
// "-" (the current size compared with it).
// Rust model: the composite is copied into an Arc for the worker (the asm borrows it: the
// modal dialog keeps the document unchanged); the job's result comes back through an
// Arc<SpinLock<JobRes>> the UI thread reads only after the job finished.  JPEG XL: the
// options are the ones the asm dialog shows (Qualität, Verlustfrei, Bemühung); the encoder
// behind them is img_jxl's (jxl-plus).  jxl-tools: JPEG XL also gets "Filmkörnung (ISO)"
// (img_jxl's photon noise, IO_FLAGS bits 24..31 = ISO / 100; 0 = off, the default).
// jxl-module: the writer is the module file jxlenc.dll / .so next to the exe; without it the
// assembly-compatible writer saves, Bemühung and Filmkörnung are off and a note says so.

use alloc::format;
use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use gfx::{Surface, FMT_PARGB};
use img_core::{ImgDec, ImgMeta, ImgOpt, ERR_CANCEL, ERR_UNSUPPORTED, IMGF_DDS, IMGF_JXL, IMGF_JXR, IMGF_PDN, IMGF_PNG, IMGF_TGA};
use img_core::{IOF_CUBE_CROSS, IOF_EMBED_EXIF, IOF_EMBED_ICC, IOF_EMBED_XMP, IOF_INTERLACE, IOF_LOSSLESS, IOF_MIPMAPS, IOF_RLE, IOF_SRGB};
use img_core::{IOM_ATHRESH, IOM_BITDEPTH, IOM_CHROMA, IOM_CUBE, IOM_DDSFMT, IOM_DITHER, IOM_EFFORT, IOM_EXIF, IOM_ICC, IOM_INTERLACE};
use img_core::{IOM_LOSSLESS, IOM_MIPFILTER, IOM_MIPMAPS, IOM_QUALITY, IOM_RLE, IOM_TIFFCOMP, IOM_XMP};
use img_core::{DDSF_A8, DDSF_B5G6R5, DDSF_BC1, DDSF_BC2, DDSF_BC3, DDSF_BC4, DDSF_BC5, DDSF_BC6H, DDSF_BC7, DDSF_BGR5A1};
use img_core::{DDSF_BGRA4, DDSF_BGRA8, DDSF_BGRX8, DDSF_L8, DDSF_R8G8, DDSF_RGBA8};
use img_core::{MIPF_BOX, MIPF_CUBIC, MIPF_LINEAR, MIPF_NEAREST, TIFC_DEFLATE, TIFC_LZW, TIFC_NONE, TIFC_PACKBITS};
use img_dds::{DDSF_BC4S, DDSF_BC5S, DDSF_BC6HS, DDSF_BGR8, IOF_DDS_UNIFORM, MIPF_FANT, MIPF_LANCZOS};
use img_jxl::writer::{IOF_JXL_NOISE_MASK, IOF_JXL_NOISE_SHIFT}; // jxl-tools (jxl-module: img_jxl::writer)
use img_webp::{IOM_PRESET, WPP_TEXT};
use rt::sync::SpinLock;
use ui::*;

use crate::app::constants::*;
use crate::document::model::DocId;
use crate::integrations::file_io::{fio_prep_opt, CMD_FIO_SOPT};
use crate::tools::SYNC_SAVEOPT;
use crate::app::Pn;

const WINID_SAVEOPT: u32 = WINID_DLG0 + 3;
const ID_SO_SCROLL: u32 = ID_DLG_SAVEOPT + 1;
const ID_SO_DEFAULTS: u32 = ID_DLG_SAVEOPT + 2;
const ID_SO_CANVAS: u32 = ID_DLG_SAVEOPT + 3;
const ID_SO_FIT: u32 = ID_DLG_SAVEOPT + 4;
const ID_SO_ACTUAL: u32 = ID_DLG_SAVEOPT + 5;
const ID_SO_BD: u32 = ID_DLG_SAVEOPT + 10; // + 0..3 radio buttons
const ID_SO_DITHER: u32 = ID_DLG_SAVEOPT + 20;
const ID_SO_THR: u32 = ID_DLG_SAVEOPT + 21;
const ID_SO_Q: u32 = ID_DLG_SAVEOPT + 22;
const ID_SO_LOSSLESS: u32 = ID_DLG_SAVEOPT + 23;
const ID_SO_CHROMA: u32 = ID_DLG_SAVEOPT + 24;
const ID_SO_PRESET: u32 = ID_DLG_SAVEOPT + 25;
const ID_SO_EFFORT: u32 = ID_DLG_SAVEOPT + 26;
const ID_SO_SPEED: u32 = ID_DLG_SAVEOPT + 27;
const ID_SO_RLE: u32 = ID_DLG_SAVEOPT + 30;
const ID_SO_TIFFC: u32 = ID_DLG_SAVEOPT + 31;
const ID_SO_DDSFMT: u32 = ID_DLG_SAVEOPT + 32;
const ID_SO_METRIC: u32 = ID_DLG_SAVEOPT + 33;
const ID_SO_CUBE: u32 = ID_DLG_SAVEOPT + 35;
const ID_SO_MIPS: u32 = ID_DLG_SAVEOPT + 36;
const ID_SO_MIPF: u32 = ID_DLG_SAVEOPT + 37;
const ID_SO_INTERL: u32 = ID_DLG_SAVEOPT + 38;
const ID_SO_NOISE: u32 = ID_DLG_SAVEOPT + 39; // jxl-tools
const ID_SO_PROBE: u32 = ID_DLG_SAVEOPT + 100;

const SETTINGS_W: i32 = 290; // DIP
/// jxl-module: the note under the JPEG XL options when the writer module (jxlenc.dll / .so
/// next to the exe) is missing
pub(crate) const JXL_COMPAT_NOTE: &str =
    "Hinweis: Das JPEG-XL-Modul (jxlenc) fehlt. Gespeichert wird mit dem einfachen Schreiber: nur Qualität und Verlustfrei wirken.";
const HDR_CAP: usize = 96;
const CHECK_DIP: i32 = 8; // checkerboard cell
const DRAG_DIP: i32 = 4; // a press that moves less is a click

// widget values v[V_*] (i32)
const V_BD: usize = 0; // bit depth: index into the format's list
const V_DITHER: usize = 1; // 0..8
const V_THR: usize = 2; // 0..255
const V_Q: usize = 3; // 1..100
const V_LOSSLESS: usize = 4;
const V_CHROMA: usize = 5; // 0 4:2:0, 1 4:2:2, 2 4:4:4
const V_PRESET: usize = 6; // WPP_*
const V_EFFORT: usize = 7; // 1..9
const V_SPEED: usize = 8; // DDS 0 fast, 1 normal, 2 slow
const V_RLE: usize = 9;
const V_TIFFC: usize = 10; // 0 LZW, 1 none, 2 Deflate, 3 PackBits
const V_DDSFMT: usize = 11; // index into DDS_VALS
const V_METRIC: usize = 12; // 0 perceptual, 1 uniform
const V_CUBE: usize = 13;
const V_MIPS: usize = 14;
const V_MIPF: usize = 15; // index into MIPF_VALS
const V_INTERL: usize = 16;
const V_NOISE: usize = 17; // jxl-tools: JPEG XL photon noise ISO (0 = off)
const NV: usize = 18;
const DDS_N: usize = 27;
const MIPF_N: usize = 6;

const IOM_UI: u32 = !(IOM_ICC | IOM_EXIF | IOM_XMP); // options with a control

const RANGE_DITHER: Range = Range::new(0, 8, 1, 0);
const RANGE_THR: Range = Range::new(0, 255, 1, 0);
const RANGE_Q: Range = Range::new(1, 100, 1, 0);
const RANGE_EFFORT: Range = Range::new(1, 9, 1, 0);
const RANGE_NOISE: Range = Range::new(0, 25500, 100, 0); // jxl-tools
static CHROMA_VALS: [u32; 4] = [420, 422, 444, 420];
static SPEED_VALS: [u32; 4] = [2, 5, 8, 5];
static TIFFC_VALS: [u32; 4] = [TIFC_LZW, TIFC_NONE, TIFC_DEFLATE, TIFC_PACKBITS];
// IO_FLAGS bit, IOM bit, v index
static FLAG_MAP: [(u32, u32, usize); 6] = [
    (IOF_LOSSLESS, IOM_LOSSLESS, V_LOSSLESS),
    (IOF_RLE, IOM_RLE, V_RLE),
    (IOF_MIPMAPS, IOM_MIPMAPS, V_MIPS),
    (IOF_CUBE_CROSS, IOM_CUBE, V_CUBE),
    (IOF_INTERLACE, IOM_INTERLACE, V_INTERL),
    (IOF_DDS_UNIFORM, IOM_DDSFMT, V_METRIC),
];
// bit depth lists: values, labels (TGA and JPEG XR: no 8-bit)
static BD_FULL: ([u32; 4], [&str; 4]) = ([0, 32, 24, 8], ["Automatisch", "32-Bit", "24-Bit", "8-Bit"]);
static BD_TGA: ([u32; 3], [&str; 3]) = ([0, 32, 24], ["Automatisch", "32-Bit", "24-Bit"]);
static CHROMA_NAMES: [&str; 3] = ["4:2:0 (kleinste Datei)", "4:2:2", "4:4:4 (höchste Qualität)"];
static PRESET_NAMES: [&str; 6] = ["Standard", "Bild", "Foto", "Zeichnen", "Icon", "Text"];
static SPEED_NAMES: [&str; 3] = ["Schnell", "Mittel", "Langsam"];
static METRIC_NAMES: [&str; 2] = ["Perzeptiv", "Einheitlich"];
static TIFFC_NAMES: [&str; 4] = ["LZW", "Keine", "Deflate (ZIP)", "PackBits"];
// DDS formats: DDSF | sRGB << 8, the names in the same order
static DDS_VALS: [u32; DDS_N] = [
    DDSF_BC1,
    DDSF_BC1 | 0x100,
    DDSF_BC2,
    DDSF_BC2 | 0x100,
    DDSF_BC3,
    DDSF_BC3 | 0x100,
    DDSF_BC4,
    DDSF_BC4S,
    DDSF_BC5,
    DDSF_BC5S,
    DDSF_BC6H,
    DDSF_BC6HS,
    DDSF_BC7,
    DDSF_BC7 | 0x100,
    DDSF_BGRA8,
    DDSF_BGRA8 | 0x100,
    DDSF_BGRX8,
    DDSF_BGRX8 | 0x100,
    DDSF_RGBA8,
    DDSF_RGBA8 | 0x100,
    DDSF_BGR5A1,
    DDSF_BGRA4,
    DDSF_B5G6R5,
    DDSF_BGR8,
    DDSF_L8,
    DDSF_A8,
    DDSF_R8G8,
];
static DDS_NAMES: [&str; DDS_N] = [
    "BC1 (Linear, DXT1)",
    "BC1 (sRGB, DX 10+)",
    "BC2 (Linear, DXT3)",
    "BC2 (sRGB, DX 10+)",
    "BC3 (Linear, DXT5)",
    "BC3 (sRGB, DX 10+)",
    "BC4 (Linear, vorzeichenlos)",
    "BC4 (Linear, mit Vorzeichen)",
    "BC5 (Linear, vorzeichenlos)",
    "BC5 (Linear, mit Vorzeichen)",
    "BC6H (Linear, vorzeichenlos, DX 11+)",
    "BC6H (Linear, mit Vorzeichen, DX 11+)",
    "BC7 (Linear, DX 11+)",
    "BC7 (sRGB, DX 11+)",
    "B8G8R8A8 (Linear, A8R8G8B8)",
    "B8G8R8A8 (sRGB, DX 10+)",
    "B8G8R8X8 (Linear, X8R8G8B8)",
    "B8G8R8X8 (sRGB, DX 10+)",
    "R8G8B8A8 (Linear, A8B8G8R8)",
    "R8G8B8A8 (sRGB, DX 10+)",
    "B5G5R5A1 (Linear, A1R5G5B5)",
    "B4G4R4A4 (Linear, A4R4G4B4)",
    "B5G6R5 (Linear, R5G6B5)",
    "B8G8R8 (Linear, R8G8B8)",
    "L8 (Linear, Helligkeit)",
    "A8 (Linear, Alpha)",
    "R8G8 (Linear, vorzeichenlos, A8L8)",
];
static MIPF_VALS: [u32; MIPF_N] = [MIPF_FANT, MIPF_CUBIC, MIPF_LINEAR, MIPF_LANCZOS, MIPF_NEAREST, MIPF_BOX];
static MIPF_NAMES: [&str; MIPF_N] = ["Fant", "Bikubisch", "Bilinear", "Lanczos", "Nächster Nachbar", "Rechteck (Box)"];

/// the result of a preview job (g_so_jbuf / jlen / jerr / jperr / jprev)
#[derive(Default)]
struct JobRes {
    buf: Option<Vec<u8>>,
    err: i32,
    perr: i32,
    prev: Surface,
}

/// the preview canvas draw callback (so_draw): borrows the app at render time
struct SoDraw {
    p: Weak<RefCell<Pn>>,
}

impl Draw for SoDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let Some(rc) = self.p.upgrade() else { return };
        let Ok(mut p) = rc.try_borrow_mut() else { return };
        so_draw(&mut p.saveopt, g, r);
    }
}

/// the state of dlg_saveopt.s (its g_* globals)
pub(crate) struct SaveOpt {
    doc: Option<DocId>,
    /// g_so_buf: result of the shown preview: file bytes (None: taken / none)
    buf: Option<Vec<u8>>,
    len: u64,
    /// g_so_plen: size of the result before (-1 none; test probe)
    plen: i64,
    job: plat::Job,
    jres: Arc<SpinLock<JobRes>>,
    /// g_so_opt: current options (no metadata)
    opt: ImgOpt<'static>,
    /// g_so_src: the composite (a copy for the worker)
    src: Arc<Surface>,
    /// g_so_prev: decoded preview; g_so_fit: preview resampled to the canvas
    prev: Surface,
    fit: Surface,
    v: [i32; NV],
    /// canvas rectangle (px) of the last draw
    crect: [i32; 4],
    fmt: u32,
    iom: u32,
    open: Shared<i32>,
    pending: bool,
    /// bumped on every option change; options version of the shown result (-1 none); of the
    /// job
    ver: u32,
    rver: u32,
    jver: u32,
    /// a job was started and not collected yet
    jact: bool,
    cancel: Arc<AtomicU32>,
    err: i32,
    perr: i32,
    fitver: u32,
    /// canvas version
    cver: u32,
    /// 0 Ganzes Bild, 1 Originalgröße
    zoom: i32,
    panx: i32,
    pany: i32,
    drag: bool,
    moved: bool,
    dx: i32,
    dy: i32,
    dx0: i32,
    dy0: i32,
    /// fitted image rectangle of the last draw (px)
    fx: i32,
    fy: i32,
    fw: i32,
    fh: i32,
    hdr: String,
    rect: Shared<WinRect>,
    draw: Option<Rc<SoDraw>>,
    /// ui.dp(CHECK_DIP) of the frame (the draw callback has no Ui)
    cell: i32,
    /// jxl-module: JPEG XL is saved with the assembly-compatible writer (the writer module
    /// jxlenc is missing): the note, Bemühung and Filmkörnung off
    jxl_compat: bool,
}

impl Default for SaveOpt {
    fn default() -> SaveOpt {
        SaveOpt {
            doc: None,
            buf: None,
            len: 0,
            plen: 0,
            job: plat::Job::new(),
            jres: Arc::new(SpinLock::new(JobRes::default())),
            opt: ImgOpt::default(),
            src: Arc::new(Surface::default()),
            prev: Surface::default(),
            fit: Surface::default(),
            v: [0; NV],
            crect: [0; 4],
            fmt: 0,
            iom: 0,
            open: Shared::new(0),
            pending: false,
            ver: 0,
            rver: 0,
            jver: 0,
            jact: false,
            cancel: Arc::new(AtomicU32::new(0)),
            err: 0,
            perr: 0,
            fitver: 0,
            cver: 0,
            zoom: 0,
            panx: 0,
            pany: 0,
            drag: false,
            moved: false,
            dx: 0,
            dy: 0,
            dx0: 0,
            dy0: 0,
            fx: 0,
            fy: 0,
            fw: 0,
            fh: 0,
            hdr: String::new(),
            rect: Shared::new(WinRect::new(0, 0, 820, 620, 0)),
            draw: None,
            jxl_compat: false,
            cell: 0,
        }
    }
}

// sync_flag() -- tool::SYNC_SAVEOPT: the script `sync` waits while the preview of the current
// options is not shown yet
fn sync_flag(s: &SaveOpt) {
    SYNC_SAVEOPT.store(s.open.get() != 0 && (s.ver != s.rver || s.jact), Ordering::SeqCst);
}

// ------------------------------------------------------------------ open / close

/// saveopt_needed(doc, fmt, saveas) -> true = show the dialog before saving
pub(crate) fn saveopt_needed(p: &Pn, doc: DocId, fmt: u32, saveas: bool) -> bool {
    if fmt == IMGF_PDN {
        return false;
    }
    let Some(fi) = img_core::io::format_info(fmt) else { return false };
    if fi.iom & IOM_UI == 0 {
        return false;
    }
    if saveas {
        return true;
    }
    let d = p.doc.d(doc);
    !(d.flags & DF_OPTOK as u32 != 0 && d.saveopt.fmt == fmt)
}

/// saveopt_open(doc, fmt) -- the document's options for fmt (paint.net's defaults when it has
/// none for this format), the widget values, the window; the first frame starts the job
pub(crate) fn saveopt_open(p: &mut Pn, ui: &mut Ui, doc: DocId, fmt: u32) {
    saveopt_close(p, ui); // nothing may be left of an earlier one
    let iom = img_core::io::format_info(fmt).map_or(0, |fi| fi.iom);
    // options: the document's when they are for this format, else the defaults
    let mut opt = img_core::io::opt_init(fmt);
    let dso = p.doc.d(doc).saveopt;
    if dso.fmt == fmt {
        opt = dso;
    }
    // the composite (a copy for the worker; the modal dialog keeps the document unchanged)
    crate::document::composite::comp_update(p, doc);
    let c = crate::document::composite::comp_sf(p.doc.d(doc));
    let src = Surface { px: c.px.clone(), w: c.w, h: c.h, stride: c.stride, fmt: c.fmt, flags: c.flags, opac: c.opac, blend: c.blend };
    let s = &mut p.saveopt;
    s.doc = Some(doc);
    s.fmt = fmt;
    s.iom = iom;
    // jxl-module: which writer saves JPEG XL (the first JPEG XL save loads the module)
    s.jxl_compat = fmt == IMGF_JXL && img_jxl::writer::kind() == img_jxl::writer::Kind::Compat;
    s.opt = opt;
    so_clean_opt(s);
    let o = s.opt;
    so_from_opt(s, &o);
    s.src = Arc::new(src);
    s.rver = u32::MAX;
    s.plen = -1;
    s.ver = s.ver.wrapping_add(1);
    s.zoom = 0;
    s.panx = 0;
    s.pany = 0;
    s.open.set(1);
    s.pending = true;
    sync_flag(s);
    ui.repass();
}

/// saveopt_close() -- cancel and wait for the job, free buffers and surfaces
pub(crate) fn saveopt_close(p: &mut Pn, _ui: &mut Ui) {
    let s = &mut p.saveopt;
    if s.jact {
        s.cancel.store(1, Ordering::SeqCst);
        s.job.cancel();
        let _ = s.job.wait();
        so_collect(s);
    }
    s.buf = None;
    s.len = 0;
    s.prev = Surface::default();
    s.fit = Surface::default();
    s.src = Arc::new(Surface::default());
    s.rver = u32::MAX;
    s.open.set(0);
    s.pending = false;
    sync_flag(s);
}

// so_clean_opt() -- opt: fmt = the dialog's format, no metadata, no cancel flag
fn so_clean_opt(s: &mut SaveOpt) {
    s.opt.fmt = s.fmt;
    s.opt.flags &= !(IOF_EMBED_ICC | IOF_EMBED_EXIF | IOF_EMBED_XMP);
    s.opt.meta = None;
    s.opt.cancel = None;
}

// ------------------------------------------------------------------ options <-> widgets

// so_bd_list() -> the bit-depth values and labels of the format (TGA and JPEG XR: no 8-bit)
fn so_bd_list(fmt: u32) -> (&'static [u32], &'static [&'static str]) {
    if fmt == IMGF_TGA || fmt == IMGF_JXR {
        (&BD_TGA.0, &BD_TGA.1)
    } else {
        (&BD_FULL.0, &BD_FULL.1)
    }
}

// so_from_opt(opt) -- the widget values from an ImgOpt
fn so_from_opt(s: &mut SaveOpt, o: &ImgOpt) {
    let fmt = s.fmt;
    let v = &mut s.v;
    // bit depth: index in the format's list (unknown -> 0 = Automatisch)
    let (bd, _) = so_bd_list(fmt);
    v[V_BD] = bd.iter().position(|&b| b == o.bitdepth).unwrap_or(0) as i32;
    // dithering level / threshold (biased: 0 = default)
    let d = o.dither.wrapping_sub(1);
    v[V_DITHER] = if d <= 8 { d as i32 } else { 7 };
    let t = o.athresh.wrapping_sub(1);
    v[V_THR] = if t <= 255 { t as i32 } else { 128 };
    v[V_Q] = if o.quality.wrapping_sub(1) < 100 { o.quality as i32 } else { so_qdef(fmt) };
    let f = o.flags;
    v[V_LOSSLESS] = (f & IOF_LOSSLESS) as i32;
    v[V_RLE] = (f >> 1 & 1) as i32;
    v[V_MIPS] = (f >> 2 & 1) as i32;
    v[V_CUBE] = (f >> 4 & 1) as i32;
    v[V_INTERL] = (f >> 8 & 1) as i32;
    v[V_METRIC] = (f >> 16 & 1) as i32;
    // chroma
    v[V_CHROMA] = match o.chroma {
        422 => 1,
        444 => 2,
        _ => 0,
    };
    // WebP preset (shares the IO_DDSFMT slot)
    v[V_PRESET] = if o.ddsfmt <= WPP_TEXT { o.ddsfmt as i32 } else { 0 };
    // effort: slider value and DDS speed class
    v[V_EFFORT] = if o.effort.wrapping_sub(1) < 9 { o.effort as i32 } else { so_edef(fmt) };
    v[V_SPEED] = match o.effort {
        0 => 1,
        1..=3 => 0,
        4..=6 => 1,
        _ => 2,
    };
    // TIFF compression: 0 = LZW (also the default 0), 1 none, 2 Deflate, 3 PackBits
    v[V_TIFFC] = match o.tiffcomp {
        TIFC_NONE => 1,
        TIFC_DEFLATE => 2,
        TIFC_PACKBITS => 3,
        _ => 0,
    };
    // DDS format + sRGB -> list index (0 when not listed)
    let dds = if o.ddsfmt == 0 { DDSF_BC1 } else { o.ddsfmt };
    let key = dds | ((f & IOF_SRGB) >> 3) << 8;
    v[V_DDSFMT] = DDS_VALS.iter().position(|&x| x == key).unwrap_or(0) as i32;
    // mip filter (0 = Fant)
    let mf = if o.mipfilter == 0 { MIPF_FANT } else { o.mipfilter };
    v[V_MIPF] = MIPF_VALS.iter().position(|&x| x == mf).unwrap_or(0) as i32;
    // jxl-tools: JPEG XL photon noise
    v[V_NOISE] = if fmt == IMGF_JXL { ((f >> IOF_JXL_NOISE_SHIFT) * 100) as i32 } else { 0 };
}

// so_to_opt(opt) -- write the widget values into opt (only the fields of the format's
// controls; every other field keeps its value)
fn so_to_opt(s: &SaveOpt, o: &mut ImgOpt) {
    let v = &s.v;
    let iom = s.iom;
    if iom & IOM_BITDEPTH != 0 {
        let (bd, _) = so_bd_list(s.fmt);
        let k = v[V_BD] as u32 as usize;
        o.bitdepth = bd[if k < bd.len() { k } else { 0 }];
    }
    if iom & IOM_DITHER != 0 {
        o.dither = (v[V_DITHER] + 1) as u32;
    }
    if iom & IOM_ATHRESH != 0 {
        o.athresh = (v[V_THR] + 1) as u32;
    }
    if iom & IOM_QUALITY != 0 {
        o.quality = v[V_Q] as u32;
    }
    if iom & IOM_CHROMA != 0 {
        o.chroma = CHROMA_VALS[(v[V_CHROMA] & 3) as usize];
    }
    if iom & IOM_PRESET != 0 {
        o.ddsfmt = v[V_PRESET] as u32;
    }
    if iom & IOM_EFFORT != 0 {
        o.effort = if s.fmt == IMGF_DDS { SPEED_VALS[(v[V_SPEED] & 3) as usize] } else { v[V_EFFORT] as u32 };
    }
    if iom & IOM_TIFFCOMP != 0 {
        o.tiffcomp = TIFFC_VALS[(v[V_TIFFC] & 3) as usize];
    }
    if iom & IOM_DDSFMT != 0 {
        let k = v[V_DDSFMT] as u32 as usize;
        let x = DDS_VALS[if k < DDS_N { k } else { 0 }];
        o.ddsfmt = x & 255;
        o.flags &= !IOF_SRGB;
        if x & 0x100 != 0 {
            o.flags |= IOF_SRGB;
        }
    }
    if iom & IOM_MIPFILTER != 0 {
        let k = v[V_MIPF] as u32 as usize;
        o.mipfilter = MIPF_VALS[if k < MIPF_N { k } else { 0 }];
    }
    // jxl-tools: JPEG XL photon noise (ISO / 100 in the flags' top byte)
    if s.fmt == IMGF_JXL {
        o.flags = o.flags & !IOF_JXL_NOISE_MASK | ((v[V_NOISE].clamp(0, 25500) / 100) as u32) << IOF_JXL_NOISE_SHIFT;
    }
    // flags: (bit, IOM, value index)
    for &(bit, m, k) in FLAG_MAP.iter() {
        if iom & m == 0 {
            continue;
        }
        o.flags &= !bit;
        if v[k] != 0 {
            o.flags |= bit;
        }
    }
}

// so_qdef() -> default quality of the format (paint.net: 95, JPEG XL / XR 90)
fn so_qdef(fmt: u32) -> i32 {
    let q = img_core::io::opt_init(fmt).quality;
    if q != 0 { q as i32 } else { 95 }
}

// so_edef() -> default effort shown for 0: PNG deflate level 6, JPEG XL 7, else 5
fn so_edef(fmt: u32) -> i32 {
    match fmt {
        IMGF_PNG => 6,
        IMGF_JXL => 7,
        _ => 5,
    }
}

// so_defaults() -- "Standardwerte": paint.net's defaults of the format into the widgets
fn so_defaults(s: &mut SaveOpt, ui: &mut Ui) {
    let o = img_core::io::opt_init(s.fmt);
    so_from_opt(s, &o);
    ui.repass();
}

// opt_eq(a, b) -- the 64-byte IMGOPT records are equal (mem_eq; meta / cancel are None here)
fn opt_eq(a: &ImgOpt, b: &ImgOpt) -> bool {
    a.fmt == b.fmt
        && a.quality == b.quality
        && a.flags == b.flags
        && a.bitdepth == b.bitdepth
        && a.samplebits == b.samplebits
        && a.dither == b.dither
        && a.athresh == b.athresh
        && a.chroma == b.chroma
        && a.effort == b.effort
        && a.ddsfmt == b.ddsfmt
        && a.mipfilter == b.mipfilter
        && a.tiffcomp == b.tiffcomp
}

// so_sync_opt() -- after the widgets: options changed -> new version (the job restarts)
fn so_sync_opt(p: &mut Pn, ui: &mut Ui) {
    let s = &mut p.saveopt;
    let mut o = s.opt;
    so_to_opt(s, &mut o);
    if opt_eq(&o, &s.opt) {
        return;
    }
    s.opt = o;
    s.ver = s.ver.wrapping_add(1);
    saveopt_poll(p, ui);
    ui.repass();
}

// ------------------------------------------------------------------ the preview job

/// saveopt_poll() -- UI thread: a finished job is collected; a job with old options is
/// cancelled; without a job and with a stale result a new one starts
pub(crate) fn saveopt_poll(p: &mut Pn, ui: &mut Ui) {
    let s = &mut p.saveopt;
    if s.jact {
        if !s.job.done() {
            if s.ver != s.jver && s.cancel.load(Ordering::SeqCst) == 0 {
                s.cancel.store(1, Ordering::SeqCst);
                s.job.cancel();
            }
            sync_flag(s);
            return;
        }
        so_collect(s);
        ui.repass();
    }
    if s.open.get() != 0 && s.ver != s.rver {
        so_start(p);
    }
    sync_flag(&p.saveopt);
}

/// saveopt_pending() -> true while the preview of the current options is not shown yet
/// (waits for a running job first; the script `sync` asks again)
pub(crate) fn saveopt_pending(p: &mut Pn, ui: &mut Ui) -> bool {
    if p.saveopt.open.get() == 0 {
        return false;
    }
    if p.saveopt.jact {
        let _ = p.saveopt.job.wait();
    }
    saveopt_poll(p, ui);
    let s = &p.saveopt;
    s.ver != s.rver || s.jact
}

// so_start() -- a job for the current options (+ metadata), jver
fn so_start(p: &mut Pn) {
    let Some(doc) = p.saveopt.doc else { return };
    let mut opt = p.saveopt.opt;
    let mut meta = ImgMeta::default();
    let use_meta = fio_prep_opt(p.doc.d(doc), &mut opt, &mut meta);
    let s = &mut p.saveopt;
    s.cancel.store(0, Ordering::SeqCst);
    s.jver = s.ver;
    *s.jres.lock() = JobRes { buf: None, err: ERR_UNSUPPORTED, perr: ERR_UNSUPPORTED, prev: Surface::default() };
    let src = s.src.clone();
    let cancel = s.cancel.clone();
    let res = s.jres.clone();
    let fmt = s.fmt;
    opt.meta = None;
    opt.cancel = None;
    s.job.set(1, move |_, _| so_worker(&src, opt, use_meta.then_some(&meta), &cancel, fmt, &res));
    s.jact = true;
    s.job.start();
}

// so_worker(src, opt, meta, cancel, fmt, res) -- worker: encode the composite with the
// options, decode the bytes
fn so_worker<'a>(src: &Surface, opt: ImgOpt<'a>, meta: Option<&'a ImgMeta>, cancel: &'a AtomicU32, fmt: u32, res: &SpinLock<JobRes>) {
    let mut o = opt;
    o.meta = meta;
    o.cancel = Some(cancel);
    let r = img_core::io::encode(src, Some(&o));
    let mut j = res.lock();
    match r {
        Err(e) => j.err = e,
        Ok(b) => {
            j.err = 0;
            let dec = ImgDec { hint: fmt, cancel: Some(cancel), ..ImgDec::default() };
            match img_core::io::decode_ex(&b, Some(&dec), None) {
                Ok(sf) => {
                    j.prev = sf;
                    j.perr = 0;
                }
                Err(e) => j.perr = e,
            }
            j.buf = Some(b);
        }
    }
}

// so_collect() -- UI thread, the job finished: its result replaces the shown one, unless it
// was cancelled (then it is dropped)
fn so_collect(s: &mut SaveOpt) {
    s.jact = false;
    let mut j = s.jres.lock();
    if j.err == ERR_CANCEL || j.perr == ERR_CANCEL {
        *j = JobRes::default();
        return;
    }
    // size before (test probe)
    s.plen = if s.rver != u32::MAX && s.err == 0 { s.len as i64 } else { -1 };
    let b = j.buf.take();
    s.len = b.as_ref().map_or(0, |b| b.len() as u64);
    s.buf = b;
    s.err = j.err;
    s.perr = j.perr;
    s.prev = core::mem::take(&mut j.prev);
    s.rver = s.jver;
    s.cver = s.cver.wrapping_add(1);
}

/// saveopt_take() -> the encoded file for the current options | ERR_*; the options become the
/// document's
pub(crate) fn saveopt_take(p: &mut Pn, _ui: &mut Ui) -> Result<Vec<u8>, i32> {
    if p.saveopt.jact {
        let _ = p.saveopt.job.wait();
        so_collect(&mut p.saveopt);
    }
    // the document keeps the options (paint.net: Speichern asks no more)
    let Some(doc) = p.saveopt.doc else { return Err(ERR_UNSUPPORTED) };
    let opt = p.saveopt.opt;
    {
        let d = p.doc.dm(doc);
        d.saveopt = opt;
        d.flags |= DF_OPTOK as u32;
    }
    let s = &mut p.saveopt;
    if s.ver == s.rver {
        if s.err != 0 {
            return Err(s.err);
        }
        s.len = 0;
        return Ok(s.buf.take().unwrap_or_default());
    }
    let mut o = opt;
    let mut meta = ImgMeta::default();
    let use_meta = fio_prep_opt(p.doc.d(doc), &mut o, &mut meta);
    if use_meta {
        o.meta = Some(&meta);
    }
    img_core::io::encode(&p.saveopt.src, Some(&o))
}

// ------------------------------------------------------------------ the window

/// saveopt_frame() -- the dialog window; a window closed by × / Esc cancels
pub(crate) fn saveopt_frame(p: &mut Pn, ui: &mut Ui) {
    if !p.saveopt.pending {
        return;
    }
    if p.saveopt.open.get() == 0 {
        p.saveopt.pending = false;
        ui.cmd_run(CMD_FIO_SOPT, DLG_CLOSE);
        return;
    }
    saveopt_poll(p, ui);
    let open = p.saveopt.open.clone();
    let rect = p.saveopt.rect.clone();
    if ui.window_begin(WINID_SAVEOPT, crate::preferences::localization::tr("Speicheroptionen"), Some(&open), WF_DIALOG | WF_MODAL | WF_RESIZABLE, Some(&rect)) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_H, 16);
        // ---- left: Einstellungen
        ui.next_size(SETTINGS_W, -1);
        ui.box_begin(0, BOX_V, 8);
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Einstellungen"));
        ui.next_size(-1, -1);
        ui.scroll_begin(ID_SO_SCROLL, SC_V);
        ui.next_size(SETTINGS_W, 0); // a fixed width: the rows settle in one pass
        ui.box_begin(0, BOX_V, 12);
        so_rows(&mut p.saveopt, ui);
        ui.box_end();
        ui.scroll_end();
        if ui.button(ID_SO_DEFAULTS, crate::preferences::localization::tr("Standardwerte")) & EV_ACT != 0 {
            so_defaults(&mut p.saveopt, ui);
        }
        ui.box_end();
        // ---- right: Vorschau
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V, 8);
        so_header(&mut p.saveopt);
        let hdr = p.saveopt.hdr.clone();
        ui.text(TS_BODY_STRONG | TX_ELLIPSIS, &hdr);
        so_canvas(p, ui);
        ui.box_begin(0, BOX_H, 16);
        let s = &mut p.saveopt;
        if ui.radio(ID_SO_FIT, "Ganzes Bild", &mut s.zoom, 0) & EV_ACT != 0 {
            s.cver = s.cver.wrapping_add(1);
        }
        if ui.radio(ID_SO_ACTUAL, "Originalgröße", &mut s.zoom, 1) & EV_ACT != 0 {
            s.cver = s.cver.wrapping_add(1);
        }
        ui.box_end();
        ui.box_end();
        ui.box_end();
        so_probe(&p.saveopt, ui);
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, Some(crate::preferences::localization::tr("Abbrechen")));
        if r == DLG_PRIMARY || r == DLG_CLOSE {
            p.saveopt.pending = false;
            ui.cmd_run(CMD_FIO_SOPT, r);
        }
    }
    ui.window_end();
    if p.saveopt.open.get() != 0 {
        so_sync_opt(p, ui);
    }
}

// so_rows() -- the controls of the format (IFI_IOM order of paint.net's dialogs)
fn so_rows(s: &mut SaveOpt, ui: &mut Ui) {
    let iom = s.iom;
    let (bd, bdl) = so_bd_list(s.fmt);
    let v = &mut s.v;
    // 8-bit options are live with "Automatisch" or 8 bit (GIF: always)
    let mut dis = false;
    if iom & IOM_BITDEPTH != 0 {
        let k = v[V_BD] as u32 as usize;
        if k < bd.len() {
            let b = bd[k];
            if b != 0 && b != 8 {
                dis = true;
            }
        }
    }
    // ---- Bit-Tiefe (radio buttons)
    if iom & IOM_BITDEPTH != 0 {
        ui.text(TS_BODY, crate::preferences::localization::tr("Bit-Tiefe"));
        for (k, l) in bdl.iter().enumerate() {
            ui.radio(ID_SO_BD + k as u32, l, &mut v[V_BD], k as i32);
        }
    }
    if iom & IOM_DITHER != 0 {
        if dis {
            ui.disabled_push();
        }
        ui.prop_slider(ID_SO_DITHER, "Dithering-Level", &mut v[V_DITHER], &RANGE_DITHER, 7);
        if dis {
            ui.disabled_pop();
        }
    }
    if iom & IOM_ATHRESH != 0 {
        if dis {
            ui.disabled_push();
        }
        ui.prop_slider(ID_SO_THR, "Schwelle für Transparenz", &mut v[V_THR], &RANGE_THR, 128);
        if dis {
            ui.disabled_pop();
        }
    }
    // ---- Qualität (off while Verlustfrei is checked)
    if iom & IOM_QUALITY != 0 {
        let off = iom & IOM_LOSSLESS != 0 && v[V_LOSSLESS] != 0;
        if off {
            ui.disabled_push();
        }
        let def = so_qdef(s.fmt);
        ui.prop_slider(ID_SO_Q, "Qualität", &mut v[V_Q], &RANGE_Q, def);
        if off {
            ui.disabled_pop();
        }
    }
    if iom & IOM_LOSSLESS != 0 {
        ui.checkbox(ID_SO_LOSSLESS, crate::preferences::localization::tr("Verlustfrei"), &mut v[V_LOSSLESS]);
    }
    // ---- Chroma-Subsampling (off while Verlustfrei is checked: always 4:4:4)
    if iom & IOM_CHROMA != 0 {
        let off = iom & IOM_LOSSLESS != 0 && v[V_LOSSLESS] != 0;
        if off {
            ui.disabled_push();
        }
        ui.text(TS_BODY, crate::preferences::localization::tr("Chroma-Subsampling"));
        ui.next_size(-1, 0);
        ui.combobox(ID_SO_CHROMA, &mut v[V_CHROMA], &CHROMA_NAMES);
        if off {
            ui.disabled_pop();
        }
    }
    if iom & IOM_PRESET != 0 {
        ui.text(TS_BODY, crate::preferences::localization::tr("Preset"));
        ui.next_size(-1, 0);
        ui.combobox(ID_SO_PRESET, &mut v[V_PRESET], &PRESET_NAMES);
    }
    // jxl-module: without the writer module the compatible writer ignores Bemühung and
    // Filmkörnung
    let jxc = s.fmt == IMGF_JXL && s.jxl_compat;
    if iom & IOM_EFFORT != 0 {
        if s.fmt == IMGF_DDS {
            ui.text(TS_BODY, crate::preferences::localization::tr("Tempo der BC6H/BC7-Kompression"));
            ui.next_size(-1, 0);
            ui.combobox(ID_SO_SPEED, &mut v[V_SPEED], &SPEED_NAMES);
        } else {
            let def = so_edef(s.fmt);
            let l = if s.fmt == IMGF_PNG { "Komprimierungsstufe" } else { "Bemühung" };
            if jxc {
                ui.disabled_push();
            }
            ui.prop_slider(ID_SO_EFFORT, l, &mut v[V_EFFORT], &RANGE_EFFORT, def);
            if jxc {
                ui.disabled_pop();
            }
        }
    }
    // ---- jxl-tools: Filmkörnung (JPEG XL photon noise; off while Verlustfrei is checked)
    if s.fmt == IMGF_JXL {
        let off = v[V_LOSSLESS] != 0 || jxc;
        if off {
            ui.disabled_push();
        }
        ui.prop_slider(ID_SO_NOISE, "Filmkörnung (ISO)", &mut v[V_NOISE], &RANGE_NOISE, 0);
        if off {
            ui.disabled_pop();
        }
    }
    // ---- jxl-module: the note when the writer module is missing
    if jxc {
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, JXL_COMPAT_NOTE);
    }
    if iom & IOM_RLE != 0 {
        ui.checkbox(ID_SO_RLE, crate::preferences::localization::tr("Komprimieren (RLE)"), &mut v[V_RLE]);
    }
    if iom & IOM_TIFFCOMP != 0 {
        ui.text(TS_BODY, crate::preferences::localization::tr("Komprimierung"));
        ui.next_size(-1, 0);
        ui.combobox(ID_SO_TIFFC, &mut v[V_TIFFC], &TIFFC_NAMES);
    }
    if iom & IOM_DDSFMT != 0 {
        ui.text(TS_BODY, crate::preferences::localization::tr("Format"));
        ui.next_size(-1, 0);
        ui.combobox(ID_SO_DDSFMT, &mut v[V_DDSFMT], &DDS_NAMES);
        ui.text(TS_BODY, crate::preferences::localization::tr("Fehlermetrik"));
        ui.next_size(-1, 0);
        ui.combobox(ID_SO_METRIC, &mut v[V_METRIC], &METRIC_NAMES);
    }
    if iom & IOM_CUBE != 0 {
        ui.checkbox(ID_SO_CUBE, crate::preferences::localization::tr("Cubemap aus Kreuz-Layout"), &mut v[V_CUBE]);
    }
    if iom & IOM_MIPMAPS != 0 {
        ui.checkbox(ID_SO_MIPS, crate::preferences::localization::tr("Mip Maps generieren"), &mut v[V_MIPS]);
        if iom & IOM_MIPFILTER != 0 {
            let off = v[V_MIPS] == 0;
            if off {
                ui.disabled_push();
            }
            ui.text(TS_BODY, crate::preferences::localization::tr("Resampling"));
            ui.next_size(-1, 0);
            ui.combobox(ID_SO_MIPF, &mut v[V_MIPF], &MIPF_NAMES);
            if off {
                ui.disabled_pop();
            }
        }
    }
    if iom & IOM_INTERLACE != 0 {
        ui.checkbox(ID_SO_INTERL, crate::preferences::localization::tr("Zeilensprung"), &mut v[V_INTERL]);
    }
}

// so_header() -- hdr = "Vorschau – Dateigröße 1,23 MB" / "(wird berechnet)" / "(Fehler)"
fn so_header(s: &mut SaveOpt) {
    let t = if s.ver != s.rver {
        String::from("(wird berechnet)")
    } else if s.err != 0 {
        String::from("(Fehler)")
    } else {
        so_size_text(s.len)
    };
    s.hdr = crate::app::formatting::fmt_cap(format!("Vorschau – Dateigröße {}", t), HDR_CAP);
}

// so_size_text(bytes) -- "123 Bytes", "45,6 KB", "1,23 MB"
fn so_size_text(n: u64) -> String {
    if n < 1024 {
        return format!("{} Bytes", n as i32);
    }
    if n < 1_048_576 {
        let t = (n * 10 + 512) >> 10; // tenths of a KB, rounded
        return format!("{},{} KB", (t / 10) as i32, (t % 10) as i32);
    }
    let h = (n * 100 + 524_288) >> 20; // hundredths of a MB, rounded
    format!("{},{:02} MB", (h / 100) as i32, (h % 100) as i32)
}

// so_canvas() -- the preview canvas: a click toggles Ganzes Bild / Originalgröße, a drag pans
// the original size
fn so_canvas(p: &mut Pn, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.next_flags(CUR_HAND << HB_CURSOR_SHIFT);
    if p.saveopt.draw.is_none() {
        p.saveopt.draw = Some(Rc::new(SoDraw { p: p.me.clone() }));
    }
    let s = &mut p.saveopt;
    s.cell = ui.dp(CHECK_DIP);
    let dr: Option<DrawRef> = s.draw.clone().map(|d| d as DrawRef);
    let inp = ui.canvas(ID_SO_CANVAS, dr, 0, s.cver).clone();
    if inp.pressed & (1 << BTN_LEFT) != 0 {
        s.dx = inp.x;
        s.dx0 = inp.x;
        s.dy = inp.y;
        s.dy0 = inp.y;
        s.drag = true;
        s.moved = false;
        return;
    }
    if !s.drag {
        return;
    }
    if inp.btn & (1 << BTN_LEFT) != 0 {
        // moved more than DRAG_DIP from the press: a drag
        let lim = ui.dp(DRAG_DIP);
        let adx = (inp.x - s.dx0).wrapping_abs();
        let ady = (inp.y - s.dy0).wrapping_abs();
        if adx <= lim && ady <= lim {
            return;
        }
        s.moved = true;
        if s.zoom == 0 {
            return;
        }
        let mx = inp.x - s.dx;
        s.dx = inp.x;
        s.panx -= mx;
        let my = inp.y - s.dy;
        s.dy = inp.y;
        s.pany -= my;
        if mx | my == 0 {
            return; // no movement since the last pass
        }
        s.cver = s.cver.wrapping_add(1);
        ui.repass();
        return;
    }
    // release
    s.drag = false;
    if s.moved {
        return;
    }
    s.zoom ^= 1;
    // Originalgröße: centre the image under the click point
    if s.zoom != 0 {
        so_center_at(s);
    }
    s.cver = s.cver.wrapping_add(1);
    ui.repass();
}

// so_center_at() -- after a click in the fitted view: pan so the clicked image pixel is in the
// middle of the canvas at 100 %  (dx0 / dy0 = click, crect = canvas)
fn so_center_at(s: &mut SaveOpt) {
    if s.prev.px.is_empty() || s.fw == 0 || s.fh == 0 {
        return;
    }
    // image x = (click - fitted origin) * iw / fw
    let x = (s.dx0 + s.crect[0] - s.fx).wrapping_mul(s.prev.w) / s.fw;
    s.panx = x - ((s.crect[2] as u32 >> 1) as i32);
    let y = (s.dy0 + s.crect[1] - s.fy).wrapping_mul(s.prev.h) / s.fh;
    s.pany = y - ((s.crect[3] as u32 >> 1) as i32);
}

// so_fit(w, h) -> the surface to show in a w x h area: the decoded preview when it fits, else
// fit (Fant-resampled, rebuilt when the size or the result changed); None on failure
fn so_fit(s: &mut SaveOpt, w: i32, h: i32) -> Option<bool> {
    let (iw, ih) = (s.prev.w, s.prev.h);
    if iw <= w && ih <= h {
        return Some(false); // the preview itself
    }
    // width-limited when iw * h > ih * w
    let (fw, fh) = if (iw as u32 as u64) * (h as u32 as u64) > (ih as u32 as u64) * (w as u32 as u64) {
        (w, ((ih as u32 as u64 * w as u32 as u64) / iw as u32 as u64) as i32)
    } else {
        (((iw as u32 as u64 * h as u32 as u64) / ih as u32 as u64) as i32, h)
    };
    let fw = fw.max(1);
    let fh = fh.max(1);
    if s.fit.px.is_empty() || s.fit.w != fw || s.fit.h != fh || s.rver != s.fitver {
        s.fit = Surface::new(fw, fh, FMT_PARGB);
        if s.fit.px.is_empty() {
            return None;
        }
        if img_core::resample::resize(&s.prev, &mut s.fit, img_core::resample::RS_FANT, 0) != 0 {
            s.fit = Surface::default();
            return None;
        }
        s.fitver = s.rver;
    }
    Some(true)
}

// so_draw(rect) -- canvas callback: the preview centred over a checkerboard
fn so_draw(s: &mut SaveOpt, g: &mut Gfx, r: [i32; 4]) {
    s.crect = r;
    s.fw = 0;
    if s.prev.px.is_empty() {
        return;
    }
    let (x0, y0, use_fit, iw, ih);
    if s.zoom == 0 {
        let Some(f) = so_fit(s, r[2], r[3]) else { return };
        use_fit = f;
        let sf = if f { &s.fit } else { &s.prev };
        iw = sf.w;
        ih = sf.h;
        x0 = ((r[2] - iw) >> 1) + r[0];
        y0 = ((r[3] - ih) >> 1) + r[1];
        s.fx = x0;
        s.fy = y0;
        s.fw = iw;
        s.fh = ih;
    } else {
        use_fit = false;
        iw = s.prev.w;
        ih = s.prev.h;
        // clamp the pan to 0 .. size - view (centred when smaller)
        x0 = so_clamp_pan(iw, r[2], &mut s.panx) + r[0];
        y0 = so_clamp_pan(ih, r[3], &mut s.pany) + r[1];
    }
    g.dc.clip_push(r[0], r[1], r[2], r[3]);
    so_checker(g, s.cell, x0, y0, iw, ih);
    let sf = if use_fit { &s.fit } else { &s.prev };
    g.dc.blit(g.sf, sf, x0, y0);
    g.dc.clip_pop();
}

// so_clamp_pan(size, view, &pan) -> offset of the image inside the view
fn so_clamp_pan(size: i32, view: i32, pan: &mut i32) -> i32 {
    if size <= view {
        *pan = 0;
        return (view - size) >> 1;
    }
    let v = (*pan).min(size - view).max(0);
    *pan = v;
    -v
}

// so_checker(x, y, w, h) -- white / light grey checkerboard (cells of CHECK_DIP, the top left
// cell white) over the part of the rectangle inside the current clip
fn so_checker(g: &mut Gfx, cell: i32, x: i32, y: i32, w: i32, h: i32) {
    g.dc.set_color(0xFFFF_FFFF);
    g.dc.fill(g.sf, x, y, w, h);
    if cell <= 0 {
        return;
    }
    let clip = g.dc.clip;
    g.dc.set_color(0xFFCC_CCCC);
    // first column / row that meets the clip
    let c0 = if clip[0] - x > 0 { ((clip[0] - x) as u32 / cell as u32) as i32 } else { 0 };
    let mut row = if clip[1] - y > 0 { ((clip[1] - y) as u32 / cell as u32) as i32 } else { 0 };
    loop {
        let oy = row * cell;
        if oy >= h || y + oy >= clip[3] {
            return;
        }
        let rh = (h - oy).min(cell); // row height (last row clipped)
        let mut col = c0;
        if (c0 + row) & 1 == 0 {
            col += 1; // grey where column + row is odd
        }
        loop {
            let ox = col * cell;
            if ox >= w || x + ox >= clip[2] {
                break;
            }
            let cw = (w - ox).min(cell); // cell width (last column clipped)
            g.dc.fill(g.sf, x + ox, y + oy, cw, rh);
            col += 2;
        }
        row += 1;
    }
}

// so_probe() -- ID_SO_PROBE: value = file size (-1 none), text busy / ok / err / none
fn so_probe(s: &SaveOpt, ui: &mut Ui) {
    ui.last_reset();
    let (t, v) = if s.ver != s.rver {
        ("busy", -1)
    } else if s.err != 0 {
        ("err", -1)
    } else if s.perr != 0 {
        ("none", s.len as i32)
    } else {
        ("ok", s.len as i32)
    };
    ui.sem(K_TEXT, ID_SO_PROBE, Some(t), v);
    // + 1: the size compared with the result before
    let mut t = "-";
    if s.plen >= 0 && s.ver == s.rver && s.err == 0 {
        let n = s.len as i64;
        t = if n == s.plen {
            "gleich"
        } else if (n as u64) < (s.plen as u64) {
            "kleiner"
        } else {
            "größer"
        };
    }
    ui.sem(K_TEXT, ID_SO_PROBE + 1, Some(t), s.plen as i32);
}
