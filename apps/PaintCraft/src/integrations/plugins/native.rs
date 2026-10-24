// plugins -- PaintCraft: plugins (agent P1, M8).  Docs: apps/PaintCraft/docs/PLUGINS.md (API for
// plugin authors), ASMLIB/docs/modules/PLUGIN_LOADER.md (loader, crash guard).
//
// Module contract (main.s):
//   plug_init()             runs before fio_init / fx_init: fx_use_all + img_use_all_codecs (the
//                           built-ins keep their registry order), then every *.dll (Windows) /
//                           *.so (Linux) of <root>/Effects, <root>/FileTypes, <root>/Shapes in
//                           name order; root = --plugins DIR | $PDN_PLUGIN_DIR | the exe folder
//                           (--no-plugins: none; "--plugins=DIR" works as well).  Per file:
//                           plat_lib_open, pn_plugin_query(PNAPI), validation + copy of
//                           PLUGININFO, registration (effects: one fx group, file types:
//                           img_register_dyn, shapes: shl_add_xaml via plug_shapes_feed).
//                           --plugin-fuzz DIR: loader robustness run only.
//   plug_states()           "Plugin-Fehler…" (Effekte menu) visible while errors are listed
//   plug_frame()            hidden probe records #0x82400..0x82403
//   plug_cmd(id, arg) -> eax  145..149 test hooks, CMD_FX_PLUGIN_ERRORS (settings page 7), the
//                           plugin menu rows (a disabled plugin: message, else fx_cmd)
// Public: plug_count() -> eax records, plug_errors() -> eax records with an error,
//   plug_loaded() -> eax loaded plugins, plug_text(i) -> (PS_* (0 = no record), one line for
//   the settings page), plug_shapes_feed() (called by shapes_lib.rs).
//
// Every call into a plugin (query, prepare, render, free, probe, decode, encode, parallel_for
// items) runs under plat::guard_call, on behalf of the plugin (plat::policy::on_behalf: what
// plat does for it needs the rights of its "<name>.permissions" beside the library as well as
// the app's; docs/PERMISSIONS.md "Plugins"; a manifest that does not parse rejects the plugin).  A fault disables the plugin (PS_CRASHED; message in the
// error list, stderr), the call returns ERR_FAULT and later calls return errors without
// entering the plugin.  Effects run through per-slot trampolines (the FxDesc copies point at
// them), file types through per-slot Codec trampolines.
// Test hooks: cmd 145 i -> #0x82401 text "state|where|kind|effects|filetypes|shapes|name" (PS_*,
// FW_*, GK_*), value = PS_*;
// #0x82400 text "loaded,errors,effects,filetypes,shapes", value = records; #0x82402 text of the
// fuzz run "files,loaded,rejected,faults", value = files; cmd 146 need -> #0x82403 text =
// img_filter_list(need) with '|' between the entries (need 1 = decoders, 2 = encoders).
//                                              [apps/PaintCraft/src/plugins.s]
//
// Rust model (the plugin ABI stays the asm's, extern "win64"):
// - The trampolines and the PNAPI functions get no context, so the plugin records, the
//   effect / file type slots and the PNAPI memory blocks are statics (spin locks, atomics);
//   Plug (in Pn) keeps the UI side (probe texts, shapes, counters).
// - The asm records a plugin sees (FXARGS, SURFACE, JOB, IMGDEC, IMGMETA, IMGOPT) are built as
//   #[repr(C)] values on the host side; their addresses go to the plugin.  Memory the plugin
//   hands back (its strings, a SURFACE / IMGDEC record, the encoder buffer) is read with
//   plat::lib_read (inside its image, validated) or plat::guard_read / guard_write (any
//   address, under the crash guard).  PA_ALLOC blocks live in a table of 64-byte aligned
//   boxes keyed by address (PA_FREE / PA_REALLOC / mem_size).
// - A render gets a destination SURFACE over a private tile buffer (copied into the fx Dst
//   afterwards); FXA_SEL is 0 (the Rust TMask has no asm layout); JOB_CANCEL of FXA_JOB is the
//   cancel state when the call started (a cancel during the call turns a finished render into
//   ERR_CANCEL).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

use ::fx::{Aux, Dst, FxArgs, FxDesc, FxProp, FP_CHOICE, FP_COLOR, FP_INT, FP_PAN, FP_SEED};
use gfx::{Surface, FMT_ARGB, FMT_PARGB};
use img_core::{Codec, ImgDec, ImgMeta, ImgOpt};
use plat::GuardInfo;
use rt::sync::SpinLock;
use ui::*;

use crate::app::constants::*;
use crate::app::formatting::fmt_cap;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const PLUG_MAX: usize = 32; // plugin files
const PFX_MAX: usize = 40; // effect slots (trampolines)
const PFT_MAX: usize = (img_core::IMG_DYN_END - img_core::IMG_DYN0) as usize; // file type slots
const PSH_MAX: usize = 64; // shapes
const PATH_CAP: usize = 1024;
const NAMES_MAX: usize = 128; // files per folder
const FNAME_CAP: usize = 128;
const PLUG_FXID0: u32 = 0x10000; // FxDesc::id of plugin effects
const FX_NAME_MAX: usize = 47; // menu name bytes (fx.rs PLUG_NAME_MAX - 1)
const NAME_MAX: usize = 63;
const CHOICE_MAX: usize = 32;
const SHAPE_DATA_MAX: usize = 16383;
const EXT_MAX: usize = 15;
const PER_PLUG_FX: u32 = 16;
const PER_PLUG_FT: u32 = 4;
const PER_PLUG_SH: u32 = 32;
const XAML_CAP: usize = 65536;
const LINE_CAP: usize = 512;
const PL_FILE_CAP: usize = 160;
const PL_MSG_CAP: usize = 256;
const ID_P1_PROBE: u32 = ID_PLUG_FIRST; // 0x82400 summary
const ID_P1_REC: u32 = ID_PLUG_FIRST + 1; // record of cmd 145
const ID_P1_FUZZ: u32 = ID_PLUG_FIRST + 2; // fuzz run
const ID_P1_FILTER: u32 = ID_PLUG_FIRST + 3; // cmd 146: img_filter_list
const CMD_P1_FILTER: u32 = CMD_P1_FIRST + 1;
const FXF_PLUGIN_OK: u32 = ::fx::FXF_ADJ
    | ::fx::FXF_PREPARE
    | ::fx::FXF_SEED
    | ::fx::FXF_BANDS
    | ::fx::FXF_WHOLE
    | ::fx::FXF_NO_DIALOG
    | ::fx::FXF_SLOW;
const IFC_PLUGIN_OK: u32 = img_core::IFC_ALPHA
    | img_core::IFC_ANIM
    | img_core::IFC_PAGES
    | img_core::IFC_LOSSY
    | img_core::IFC_LOSSLESS
    | img_core::IFC_HDR;
const ERR_FAULT: i32 = plat::ERR_FAULT;
const ERR_FAIL: i32 = img_core::ERR_FAIL;

// plugin.inc
const PNAPI_VERSION: u32 = 1;
const PLUGIN_MAGIC: u32 = 0x49504E50; // "PNPI"
const PA_SIZE_V1: u32 = 96;
const PI_SIZE_V1: usize = 88;
const FXD_SIZE: usize = 64;
const FXP_SIZE: usize = 48;
const FT_SIZE: usize = 56;
const PSD_SIZE: usize = 16;
const JOB_CANCEL: usize = 48; // thread.inc

// record states (PS_*)
pub(crate) const PS_NONE: u32 = 0;
pub(crate) const PS_LOADED: u32 = 1;
pub(crate) const PS_REJECTED: u32 = 2;
pub(crate) const PS_CRASHED: u32 = 3;
const PS_BUSY: u32 = 4; // a crash is being recorded
const PS_LOADING: u32 = 5;

// where a fault happened (s_where)
const FW_LOADER: u32 = 0;
const FW_QUERY: u32 = 1;
const FW_PREPARE: u32 = 3;
const FW_RENDER: u32 = 4;
const FW_FREE: u32 = 5;
const FW_PROBE: u32 = 6;
const FW_DECODE: u32 = 7;
const FW_ENCODE: u32 = 8;
const FW_PARFOR: u32 = 9;

static S_FOLDERS: [&str; 3] = ["Effects", "FileTypes", "Shapes"];
const S_LIBEXT: &str = if cfg!(windows) { ".dll" } else { ".so" };
static S_WHERE: [&str; 10] =
    ["dem Lader", "pn_plugin_query", "den Plugin-Daten", "prepare", "render", "free", "probe", "decode", "encode", "parallel_for"];
static S_KINDS: [&str; 7] =
    ["Fehler", "Zugriffsverletzung", "ungültiger Befehl", "Rechenfehler", "Ausrichtungsfehler", "Haltepunkt", "Stapelüberlauf"];
static S_LIBE: [&str; 8] = [
    "Die Plugin-Datei ließ sich nicht laden.",
    "Die Plugin-Datei ließ sich nicht lesen.",
    "Keine gültige Plugin-Bibliothek (x86-64-DLL / ELF-Shared-Object erwartet) oder die Datei ist defekt.",
    "Das Plugin benötigt andere Bibliotheken (nicht erlaubt: nur die Plugin-API).",
    "Das Plugin enthält eine nicht unterstützte Relokation.",
    "Zu wenig Arbeitsspeicher, um das Plugin zu laden.",
    "Thread-lokale Daten, Initialisierer oder ein Interpreter werden nicht unterstützt.",
    "Der Speicher für das Plugin konnte nicht eingerichtet werden.",
];
const S_BADPTR: &str = "Ungültige Plugin-Info (Zeiger außerhalb des Plugins).";
const S_X_OPEN: &str =
    "<ps:SimpleGeometryShape xmlns:ps=\"clr-namespace:PaintDotNet.Shapes;assembly=PaintDotNet.Framework\" DisplayName=\"";
const S_X_GEO: &str = "\" Geometry=\"";
const S_X_CLOSE: &str = "\" />\n";

// ================================================================== shared state

/// a plugin record (PL_*)
#[derive(Default)]
struct PlugRec {
    lib: Option<plat::Lib>,
    folder: u32,
    nfx: u32,
    nft: u32,
    nsh: u32,
    /// FW_* of the fault, the fault
    whr: u32,
    fault: GuardInfo,
    /// "Effects/name.dll"
    file: String,
    name: String,
    ver: String,
    author: String,
    url: String,
    msg: String,
}

static PLUGS: SpinLock<Vec<PlugRec>> = SpinLock::new(Vec::new());
/// the rights of every record (plat::policy::plugin_rights; gcall runs plugin code with them)
static RIGHTS: SpinLock<Vec<Option<alloc::sync::Arc<plat::policy::PluginRights>>>> = SpinLock::new(Vec::new());
#[allow(clippy::declare_interior_mutable_const)]
const AZ: AtomicU32 = AtomicU32::new(0);
/// PL_STATE of every record (read by the trampolines on any thread)
static PSTATE: [AtomicU32; PLUG_MAX] = [AZ; PLUG_MAX];

/// an effect slot (PX_*): the plugin and its functions
#[derive(Clone, Copy, Default)]
struct PfxSlot {
    plug: usize,
    prep: u64,
    rend: u64,
    free: u64,
    name: &'static str,
}

/// a file type slot (PT_*)
#[derive(Clone, Copy, Default)]
struct PftSlot {
    plug: usize,
    probe: u64,
    dec: u64,
    enc: u64,
}

static PFX: SpinLock<Vec<PfxSlot>> = SpinLock::new(Vec::new());
static PFT: SpinLock<Vec<PftSlot>> = SpinLock::new(Vec::new());

/// a PA_ALLOC block: 64-byte aligned
#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct A64([u8; 64]);

/// PA_ALLOC blocks by address: (memory, size asked for)
static BLOCKS: SpinLock<BTreeMap<u64, (Box<[A64]>, usize)>> = SpinLock::new(BTreeMap::new());

/// the state of plugins.s the UI keeps (probe texts, shapes, counters)
#[derive(Default)]
pub(crate) struct Plug {
    root: String,
    /// g_psh: (plugin, name, path data)
    shapes: Vec<(usize, String, String)>,
    npfx: usize,
    npft: usize,
    rectxt: String,
    recstate: u32,
    fztxt: String,
    /// files, loaded, rejected, faults
    fz: [u32; 4],
    filtxt: String,
    full_logged: bool,
    /// the effects of every loaded plugin until they are registered (one fx group)
    fxdesc: Vec<FxDesc>,
}

// ================================================================== asm records

/// SURFACE (jfgui.inc SF_*)
#[repr(C)]
#[derive(Default)]
struct SurfAbi {
    px: u64,
    w: i32,
    h: i32,
    stride: i32,
    fmt: u8,
    flags: u8,
    opac: u8,
    blend: u8,
    aux: u64,
}

impl SurfAbi {
    fn bytes(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..8].copy_from_slice(&self.px.to_le_bytes());
        b[8..12].copy_from_slice(&self.w.to_le_bytes());
        b[12..16].copy_from_slice(&self.h.to_le_bytes());
        b[16..20].copy_from_slice(&self.stride.to_le_bytes());
        b[20] = self.fmt;
        b[21] = self.flags;
        b[22] = self.opac;
        b[23] = self.blend;
        b[24..32].copy_from_slice(&self.aux.to_le_bytes());
        b
    }
    fn from(b: &[u8]) -> SurfAbi {
        SurfAbi {
            px: le64(b, 0),
            w: le32(b, 8) as i32,
            h: le32(b, 12) as i32,
            stride: le32(b, 16) as i32,
            fmt: b[20],
            flags: b[21],
            opac: b[22],
            blend: b[23],
            aux: le64(b, 24),
        }
    }
}

/// FXARGS (fx.inc FXA_*)
#[repr(C)]
struct FxArgsAbi {
    src: u64,
    dst: u64,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    param: u64,
    aux: u64,
    job: u64,
    sel: u64,
}

/// JOB (thread.inc JOB_*): plugins read JOB_CANCEL
#[repr(C)]
struct JobAbi {
    r: [u64; 6],
    cancel: AtomicU32,
    state: u32,
    active: u32,
    flags: u32,
    link: u64,
    rsv: u64,
}

impl JobAbi {
    fn new(cancel: bool) -> JobAbi {
        JobAbi { r: [0; 6], cancel: AtomicU32::new(cancel as u32), state: 0, active: 0, flags: 0, link: 0, rsv: 0 }
    }
}

const _: () = assert!(core::mem::size_of::<SurfAbi>() == 32);
const _: () = assert!(core::mem::size_of::<FxArgsAbi>() == 64);
const _: () = assert!(core::mem::size_of::<JobAbi>() == 80);
const _: () = assert!(core::mem::offset_of!(JobAbi, cancel) == JOB_CANCEL);

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn le64(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

fn addr<T>(v: &T) -> u64 {
    v as *const T as u64
}

// gcall(fn, a1..a4, tag) -> (rax, rdx, fault info) -- a guarded call into plugin code
fn gcall(f: u64, a: [u64; 4], tag: u64) -> (u64, u64, GuardInfo) {
    let mut info = GuardInfo { tag, ..GuardInfo::default() };
    let rights = if tag >= 1 { RIGHTS.lock().get(tag as usize - 1).cloned().flatten() } else { None };
    let mut call = || plat::guard_call(f as usize, a[0], a[1], a[2], a[3], Some(&mut info));
    let (rax, rdx) = match rights {
        Some(r) => plat::policy::on_behalf(&r, call),
        None => call(),
    };
    (rax, rdx, info)
}

fn pstate(i: usize) -> u32 {
    PSTATE.get(i).map_or(PS_NONE, |s| s.load(SeqCst))
}

// cstr(addr, max) -> the NUL-terminated string at addr (at most max bytes; any memory, under
// the guard), lossy UTF-8
fn cstr(a: u64, max: usize) -> Option<String> {
    if a == 0 {
        return None;
    }
    let mut out: Vec<u8> = Vec::new();
    let mut p = a;
    while out.len() < max {
        // page-sized pieces that do not cross a page boundary (the string may end before
        // an unmapped page)
        let n = (4096 - (p & 4095) as usize).min(max - out.len());
        let b = plat::guard_read(p as usize, n)?;
        if let Some(k) = b.iter().position(|&c| c == 0) {
            out.extend_from_slice(&b[..k]);
            return Some(String::from_utf8_lossy(&out).into_owned());
        }
        out.extend_from_slice(&b);
        p += n as u64;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

// ================================================================== PNAPI blocks

// blk_alloc(size) -> address of a zeroed 64-byte aligned block | 0
fn blk_alloc(size: u64) -> u64 {
    let n = (size.saturating_add(63) / 64).max(1) as usize;
    let mut v: Vec<A64> = Vec::new();
    if size > (1u64 << 40) || v.try_reserve_exact(n).is_err() {
        return 0;
    }
    v.resize(n, A64([0; 64]));
    let b = v.into_boxed_slice();
    let a = b.as_ptr() as u64;
    BLOCKS.lock().insert(a, (b, size as usize));
    a
}

// blk_free(addr) -- (unknown addresses are ignored)
fn blk_free(a: u64) {
    let b = BLOCKS.lock().remove(&a);
    drop(b);
}

// blk_size(addr) -> the size asked for | None (not a block)
fn blk_size(a: u64) -> Option<usize> {
    BLOCKS.lock().get(&a).map(|e| e.1)
}

extern "win64" fn pa_alloc(size: u64) -> u64 {
    blk_alloc(size)
}

extern "win64" fn pa_free(p: u64) {
    blk_free(p)
}

extern "win64" fn pa_realloc(p: u64, size: u64) -> u64 {
    if p == 0 {
        return blk_alloc(size);
    }
    {
        let mut t = BLOCKS.lock();
        let Some(e) = t.get_mut(&p) else { return 0 };
        if size as usize <= e.0.len() * 64 {
            e.1 = size as usize; // fits
            return p;
        }
    }
    let old = blk_size(p).unwrap_or(0);
    let n = blk_alloc(size);
    if n == 0 {
        return 0; // the old block stays valid
    }
    if let Some(d) = plat::guard_read(p as usize, old.min(size as usize)) {
        plat::guard_write(n as usize, &d);
    }
    blk_free(p);
    n
}

// ================================================================== PNAPI functions

// pa_parfor(fn, ctx, count) -> 0 / ERR_CANCEL / ERR_FAULT -- parallel_for with every item
// guarded (tag of the calling plugin: a crash disables it and cancels the rest)
extern "win64" fn pa_parfor(f: u64, ctx: u64, count: u64) -> i32 {
    let tag = plat::guard_tag();
    let fault = AtomicBool::new(false);
    let job = Box::new(JobAbi::new(false));
    let ja = addr(&*job);
    let item = |i: usize, ctl: &plat::JobCtl| {
        if fault.load(SeqCst) {
            return;
        }
        let (_, _, info) = gcall(f, [ctx, i as u64, ja, 0], tag);
        if info.kind != 0 {
            fault.store(true, SeqCst);
            job.cancel.store(1, SeqCst);
            ctl.cancel();
            if tag != 0 {
                plug_crashed(tag as usize - 1, FW_PARFOR, &info);
            }
        }
    };
    let r = plat::parallel_for(count.min(1 << 40) as usize, &item);
    if fault.load(SeqCst) {
        return ERR_FAULT;
    }
    if r.is_err() { ::fx::ERR_CANCEL } else { 0 }
}

extern "win64" fn pa_hash32(x: u32, y: u32, seed: u32, stream: u32) -> u32 {
    ::fx::hash32(x, y, seed, stream)
}

// pa_log(msg) -- "PaintCraft: Plugin <file>: msg" on stderr (any thread)
extern "win64" fn pa_log(msg: u64) {
    let tag = plat::guard_tag();
    let file = if tag >= 1 && tag as usize <= PLUG_MAX {
        PLUGS.lock().get(tag as usize - 1).map_or(String::from("?"), |r| r.file.clone())
    } else {
        String::from("?")
    };
    let m = cstr(msg, LINE_CAP).unwrap_or_else(|| String::from("(null)"));
    let line = fmt_cap(format!("PaintCraft: Plugin {}: {}\n", file, m), LINE_CAP);
    plat::write_stderr(line.as_bytes());
}

// pa_row(sf, y) -> pointer to row y of sf or 0
extern "win64" fn pa_row(sf: u64, y: i32) -> u64 {
    let Some(b) = plat::guard_read(sf as usize, 32) else { return 0 };
    let s = SurfAbi::from(&b);
    if y < 0 || y >= s.h {
        return 0;
    }
    s.px.wrapping_add((s.stride as i64).wrapping_mul(y as i64) as u64)
}

// dec_of(addr) -> the scalar fields of an IMGDEC record (0: defaults)
fn dec_of(a: u64) -> ImgDec<'static> {
    let mut d = ImgDec::default();
    if let Some(b) = plat::guard_read(a as usize, 48) {
        d.flags = le32(&b, 0);
        d.frame = le32(&b, 4);
        d.hint = le32(&b, 8);
        d.maxpx = le64(&b, 16);
        d.maxmem = le64(&b, 40);
    }
    d
}

// pa_surfalloc(sf, w, h, dec) -> 0 / ERR_* -- a zeroed straight FMT_ARGB surface (limits of
// the IMGDEC) written into the plugin's SURFACE record
extern "win64" fn pa_surfalloc(sf: u64, w: i32, h: i32, dec: u64) -> i32 {
    let d = dec_of(dec);
    let r = img_core::meta::dims_check(w as u32, h as u32, &d);
    if r != 0 {
        return r;
    }
    let stride = (w as i64 * 4 + 63) & !63;
    let px = blk_alloc((stride * h as i64) as u64);
    if px == 0 {
        return img_core::ERR_NOMEM;
    }
    let rec = SurfAbi { px, w, h, stride: stride as i32, fmt: FMT_ARGB, flags: gfx::SFF_OWNED, opac: 255, blend: 0, aux: 0 };
    if !plat::guard_write(sf as usize, &rec.bytes()) {
        blk_free(px);
        return ERR_FAULT;
    }
    0
}

// pa_cancelled(job) -> 1 when the job's cancel flag is set
extern "win64" fn pa_cancelled(job: u64) -> i32 {
    match plat::guard_read((job as usize).wrapping_add(JOB_CANCEL), 4) {
        Some(b) if job != 0 => (le32(&b, 0) != 0) as i32,
        _ => 0,
    }
}

// pa_fmt(buf, cap, fmt, ...) -> length -- fmt_buf: %d %i %u %x %X %c %s %p %b %%, prefix l,
// flags '-' '0', width (digits or '*'), precision .N for %s; every argument one qword (the
// first six are taken; Win64 varargs are laid out like ordinary arguments)
#[allow(clippy::too_many_arguments)]
extern "win64" fn pa_fmt(buf: u64, cap: u64, fmt: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> u64 {
    let args = [a1, a2, a3, a4, a5, a6];
    let f = cstr(fmt, 4096).unwrap_or_default();
    let s = fmt_printf(f.as_bytes(), &args);
    if cap == 0 {
        return 0;
    }
    let out = fmt_cap(s, cap.min(1 << 20) as usize);
    let mut b = out.into_bytes();
    let n = b.len();
    b.push(0);
    plat::guard_write(buf as usize, &b);
    n as u64
}

// fmt_printf(fmt, args) -- the conversions of fmt_buf
fn fmt_printf(f: &[u8], args: &[u64]) -> String {
    let mut out = String::new();
    let mut ai = 0usize;
    let next = |ai: &mut usize| -> u64 {
        let v = args.get(*ai).copied().unwrap_or(0);
        *ai += 1;
        v
    };
    let mut i = 0;
    while i < f.len() {
        let c = f[i];
        i += 1;
        if c != b'%' {
            // copy UTF-8 runs as bytes
            let st = i - 1;
            let mut e = i;
            while e < f.len() && f[e] != b'%' {
                e += 1;
            }
            out.push_str(&String::from_utf8_lossy(&f[st..e]));
            i = e;
            continue;
        }
        let (mut left, mut zero, mut long) = (false, false, false);
        while i < f.len() && (f[i] == b'-' || f[i] == b'0') {
            if f[i] == b'-' {
                left = true;
            } else {
                zero = true;
            }
            i += 1;
        }
        let mut width = 0usize;
        if i < f.len() && f[i] == b'*' {
            width = next(&mut ai) as i32 as usize;
            i += 1;
        } else {
            while i < f.len() && f[i].is_ascii_digit() {
                width = width * 10 + (f[i] - b'0') as usize;
                i += 1;
            }
        }
        let mut prec = None;
        if i < f.len() && f[i] == b'.' {
            i += 1;
            let mut p = 0usize;
            while i < f.len() && f[i].is_ascii_digit() {
                p = p * 10 + (f[i] - b'0') as usize;
                i += 1;
            }
            prec = Some(p);
        }
        if i < f.len() && f[i] == b'l' {
            long = true;
            i += 1;
        }
        if i >= f.len() {
            break;
        }
        let conv = f[i];
        i += 1;
        let body = match conv {
            b'd' | b'i' => {
                let v = next(&mut ai);
                if long { format!("{}", v as i64) } else { format!("{}", v as u32 as i32) }
            }
            b'u' => {
                let v = next(&mut ai);
                if long { format!("{}", v) } else { format!("{}", v as u32) }
            }
            b'x' => {
                let v = next(&mut ai);
                if long { format!("{:x}", v) } else { format!("{:x}", v as u32) }
            }
            b'X' => {
                let v = next(&mut ai);
                if long { format!("{:X}", v) } else { format!("{:X}", v as u32) }
            }
            b'b' => {
                let v = next(&mut ai);
                if long { format!("{:b}", v) } else { format!("{:b}", v as u32) }
            }
            b'p' => format!("0x{:016x}", next(&mut ai)),
            b'c' => String::from(char::from_u32(next(&mut ai) as u32).unwrap_or('?')),
            b's' => {
                let v = next(&mut ai);
                let s = if v == 0 { String::from("(null)") } else { cstr(v, 4096).unwrap_or_default() };
                match prec {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                }
            }
            b'%' => String::from("%"),
            _ => String::new(),
        };
        let n = body.chars().count();
        if n < width {
            let pad = width - n;
            if left {
                out.push_str(&body);
                out.extend(core::iter::repeat_n(' ', pad));
            } else if zero && conv != b's' && conv != b'c' {
                let (sign, digits) = if let Some(d) = body.strip_prefix('-') { ("-", d) } else { ("", body.as_str()) };
                out.push_str(sign);
                out.extend(core::iter::repeat_n('0', pad));
                out.push_str(digits);
            } else {
                out.extend(core::iter::repeat_n(' ', pad));
                out.push_str(&body);
            }
        } else {
            out.push_str(&body);
        }
    }
    out
}

extern "win64" fn pa_nthreads() -> u32 {
    plat::thr_count()
}

/// PNAPI version 1 (plugin.inc)
#[repr(C)]
struct PnApi {
    size: u32,
    version: u32,
    alloc: extern "win64" fn(u64) -> u64,
    free: extern "win64" fn(u64),
    realloc: extern "win64" fn(u64, u64) -> u64,
    parfor: extern "win64" fn(u64, u64, u64) -> i32,
    hash32: extern "win64" fn(u32, u32, u32, u32) -> u32,
    log: extern "win64" fn(u64),
    row: extern "win64" fn(u64, i32) -> u64,
    surfalloc: extern "win64" fn(u64, i32, i32, u64) -> i32,
    cancelled: extern "win64" fn(u64) -> i32,
    #[allow(clippy::type_complexity)]
    fmt: extern "win64" fn(u64, u64, u64, u64, u64, u64, u64, u64, u64) -> u64,
    nthreads: extern "win64" fn() -> u32,
}

static PNAPI: PnApi = PnApi {
    size: PA_SIZE_V1,
    version: PNAPI_VERSION,
    alloc: pa_alloc,
    free: pa_free,
    realloc: pa_realloc,
    parfor: pa_parfor,
    hash32: pa_hash32,
    log: pa_log,
    row: pa_row,
    surfalloc: pa_surfalloc,
    cancelled: pa_cancelled,
    fmt: pa_fmt,
    nthreads: pa_nthreads,
};

const _: () = assert!(core::mem::size_of::<PnApi>() == PA_SIZE_V1 as usize);

// ================================================================== errors

// plug_crashed(i, where, info) -- a fault at run time (any thread): PS_LOADED -> PS_CRASHED
// once, message + stderr, wake the UI
fn plug_crashed(i: usize, whr: u32, info: &GuardInfo) {
    if i >= PLUG_MAX || PSTATE[i].compare_exchange(PS_LOADED, PS_BUSY, SeqCst, SeqCst).is_err() {
        return; // already disabled
    }
    let line = {
        let mut g = PLUGS.lock();
        let Some(r) = g.get_mut(i) else { return };
        let (base, span) = r.lib.as_ref().map_or((0, 0), |l| (l.base() as u64, l.span() as u64));
        plug_fault_msg(r, whr, info, base, span);
        fmt_cap(format!("PaintCraft: Plugin {}: {}\n", r.file, r.msg), LINE_CAP)
    };
    PSTATE[i].store(PS_CRASHED, SeqCst);
    plat::write_stderr(line.as_bytes());
    plat::wake();
}

// plug_fault_msg(rec, where, info, image) -- where, the fault and the German message
fn plug_fault_msg(r: &mut PlugRec, whr: u32, info: &GuardInfo, base: u64, span: u64) {
    r.whr = whr;
    r.fault = *info;
    // instruction address relative to the image when inside it
    let pc = info.pc;
    let pcs = if base != 0 && pc >= base && pc - base < span { format!("+0x{:x}", pc - base) } else { format!("0x{:x}", pc) };
    let pcs = fmt_cap(pcs, 32);
    let w = S_WHERE[if whr as usize <= FW_PARFOR as usize { whr as usize } else { 0 }];
    let k = S_KINDS[if info.kind as usize <= 6 { info.kind as usize } else { 0 }];
    r.msg = fmt_cap(format!("Absturz in {} ({} bei {}, Adresse 0x{:x}). Das Plugin wurde deaktiviert.", w, k, pcs, info.addr), PL_MSG_CAP);
}

// plug_reject(i, msg) -- PS_REJECTED with a message (UI thread, loading)
fn plug_reject(i: usize, msg: String) {
    let line = {
        let mut g = PLUGS.lock();
        let r = &mut g[i];
        r.msg = fmt_cap(msg, PL_MSG_CAP);
        fmt_cap(format!("PaintCraft: Plugin {}: {}\n", r.file, r.msg), LINE_CAP)
    };
    PSTATE[i].store(PS_REJECTED, SeqCst);
    plat::write_stderr(line.as_bytes());
}

// plug_log(i) -- the record's message on stderr
fn plug_log(i: usize) {
    let line = {
        let g = PLUGS.lock();
        fmt_cap(format!("PaintCraft: Plugin {}: {}\n", g[i].file, g[i].msg), LINE_CAP)
    };
    plat::write_stderr(line.as_bytes());
}

// ================================================================== effect trampolines

/// the prepare result of a plugin effect (FXD_FREE is its Drop)
struct PlugAux {
    slot: usize,
    ptr: u64,
}

impl Drop for PlugAux {
    // pfx_free(aux) -- a disabled plugin's aux is leaked, never passed back
    fn drop(&mut self) {
        let Some(s) = PFX.lock().get(self.slot).copied() else { return };
        if pstate(s.plug) != PS_LOADED || s.free == 0 {
            return;
        }
        let (_, _, info) = gcall(s.free, [self.ptr, 0, 0, 0], s.plug as u64 + 1);
        if info.kind != 0 {
            plug_crashed(s.plug, FW_FREE, &info);
        }
    }
}

// the records of one call: source, destination over buf (the ROI at stride), job
fn fx_records(a: &FxArgs, dst_px: u64, dst_stride: i32) -> (SurfAbi, SurfAbi, Box<JobAbi>) {
    let src = SurfAbi {
        px: a.src.px.as_ptr() as u64,
        w: a.src.w,
        h: a.src.h,
        stride: a.src.stride,
        fmt: FMT_ARGB,
        flags: 0,
        opac: 255,
        blend: 0,
        aux: 0,
    };
    let dst = SurfAbi { px: dst_px, w: a.src.w, h: a.src.h, stride: dst_stride, fmt: FMT_ARGB, flags: 0, opac: 255, blend: 0, aux: 0 };
    (src, dst, Box::new(JobAbi::new(a.cancelled())))
}

// pfx_prepare(slot, args) -> aux | Err(ERR_*)
fn pfx_prepare_slot(k: usize, a: &FxArgs) -> Result<Option<Aux>, i32> {
    let Some(s) = PFX.lock().get(k).copied() else { return Err(ERR_FAULT) };
    if pstate(s.plug) != PS_LOADED {
        return Err(ERR_FAULT);
    }
    let (src, dst, job) = fx_records(a, 0, a.src.w.wrapping_mul(4));
    let args = FxArgsAbi {
        src: addr(&src),
        dst: addr(&dst),
        x0: a.x0,
        y0: a.y0,
        x1: a.x1,
        y1: a.y1,
        param: a.params.0.as_ptr() as u64,
        aux: 0,
        job: addr(&*job),
        sel: 0,
    };
    let (rax, rdx, info) = gcall(s.prep, [addr(&args), 0, 0, 0], s.plug as u64 + 1);
    if info.kind != 0 {
        plug_crashed(s.plug, FW_PREPARE, &info);
        return Err(ERR_FAULT);
    }
    if rax != 0 {
        return Ok(Some(Box::new(PlugAux { slot: k, ptr: rax })));
    }
    if (rdx as i32) < 0 { Err(rdx as i32) } else { Ok(None) }
}

// pfx_render(slot, args, dst) -> 0 / ERR_*
fn pfx_render_slot(k: usize, a: &FxArgs, d: &mut Dst) -> i32 {
    let Some(s) = PFX.lock().get(k).copied() else { return ERR_FAULT };
    if pstate(s.plug) != PS_LOADED {
        return ERR_FAULT;
    }
    let (tw, th) = ((a.x1 - a.x0).max(0) as usize, (a.y1 - a.y0).max(0) as usize);
    let stride = tw * 4;
    let mut tmp = vec![0u8; stride * th];
    // the destination SURFACE: pixel (x, y) of the ROI lands in tmp
    let off = (a.y0 as i64).wrapping_mul(stride as i64).wrapping_add(a.x0 as i64 * 4);
    let base = (tmp.as_mut_ptr() as u64).wrapping_sub(off as u64);
    let (src, dst, job) = fx_records(a, base, stride as i32);
    let aux = a.aux.and_then(|x| x.downcast_ref::<PlugAux>()).map_or(0, |x| x.ptr);
    let args = FxArgsAbi {
        src: addr(&src),
        dst: addr(&dst),
        x0: a.x0,
        y0: a.y0,
        x1: a.x1,
        y1: a.y1,
        param: a.params.0.as_ptr() as u64,
        aux,
        job: addr(&*job),
        sel: 0,
    };
    let (rax, _, info) = gcall(s.rend, [addr(&args), 0, 0, 0], s.plug as u64 + 1);
    if info.kind != 0 {
        plug_crashed(s.plug, FW_RENDER, &info);
        return ERR_FAULT;
    }
    if pstate(s.plug) != PS_LOADED {
        return ERR_FAULT; // a parallel_for item of this call crashed
    }
    let r = rax as i32;
    if r != 0 {
        return r;
    }
    if a.cancelled() {
        return ::fx::ERR_CANCEL;
    }
    for y in a.y0..a.y1 {
        let o = (y - a.y0) as usize * stride;
        d.row(y).copy_from_slice(&tmp[o..o + stride]);
    }
    0
}

fn pfx_render<const K: usize>(a: &FxArgs, d: &mut Dst) -> i32 {
    pfx_render_slot(K, a, d)
}

fn pfx_prepare<const K: usize>(a: &FxArgs) -> Result<Option<Aux>, i32> {
    pfx_prepare_slot(K, a)
}

macro_rules! slot_table {
    ($f:ident; $($k:literal)*) => { [$($f::<$k>),*] };
}

static PFX_REND: [::fx::RenderFn; PFX_MAX] =
    slot_table!(pfx_render; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39);
static PFX_PREP: [::fx::PrepareFn; PFX_MAX] =
    slot_table!(pfx_prepare; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39);

// ================================================================== file type trampolines

// meta_from(b, meta) -- the scalar fields of an IMGMETA record into meta
fn meta_from(b: &[u8], m: &mut ImgMeta) {
    if b.len() < 128 {
        return;
    }
    m.fmt = le32(b, 0);
    m.w = le32(b, 4);
    m.h = le32(b, 8);
    m.bits = b[12];
    m.model = b[13];
    m.orient = b[14];
    m.channels = b[15];
    m.flags = le32(b, 16);
    m.frames = le32(b, 20);
    m.dpix = le32(b, 24);
    m.dpiy = le32(b, 28);
    m.cicp = le32(b, 32);
    m.gamma = le32(b, 36);
    m.delay = le32(b, 88);
    m.loops = le32(b, 92);
    m.quality = le32(b, 96);
    m.resunit = le32(b, 100);
    for k in 0..8 {
        m.chrm[k] = u16::from_le_bytes([b[104 + k * 2], b[105 + k * 2]]);
    }
    m.dxgi = u16::from_le_bytes([b[120], b[121]]);
    m.texfl = b[122];
    m.texn = le32(b, 124);
}

fn pft_slot(j: usize) -> Option<PftSlot> {
    let s = PFT.lock().get(j).copied()?;
    if pstate(s.plug) != PS_LOADED { None } else { Some(s) }
}

// pft_call(slot, fn, args, where) -> (rax, faulted) -- a guarded codec call; a fault
// disables the plugin
fn pft_call(s: &PftSlot, f: u64, a: [u64; 4], whr: u32) -> (u64, bool) {
    let (rax, _, info) = gcall(f, a, s.plug as u64 + 1);
    if info.kind != 0 {
        plug_crashed(s.plug, whr, &info);
        return (ERR_FAULT as i64 as u64, true);
    }
    (rax, false)
}

// pft_probe(data, meta) -> recognised
fn pft_probe_slot(j: usize, data: &[u8], meta: &mut ImgMeta) -> bool {
    let Some(s) = pft_slot(j) else { return false };
    let mb = Box::new(meta.to_bytes());
    let (rax, fault) = pft_call(&s, s.probe, [data.as_ptr() as u64, data.len() as u64, addr(&*mb), 0], FW_PROBE);
    if fault {
        return false;
    }
    meta_from(&mb[..], meta);
    rax as u32 != 0
}

// pft_decode(data, out, dec, meta) -> 0 / ERR_* -- the result must be a sane FMT_ARGB /
// FMT_PARGB surface (else ERR_FAIL)
fn pft_decode_slot(j: usize, data: &[u8], out: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    let Some(s) = pft_slot(j) else { return img_core::ERR_UNSUPPORTED };
    let mb = Box::new(meta.to_bytes());
    let mut db = Box::new([0u8; 48]);
    db[0..4].copy_from_slice(&dec.flags.to_le_bytes());
    db[4..8].copy_from_slice(&dec.frame.to_le_bytes());
    db[8..12].copy_from_slice(&dec.hint.to_le_bytes());
    db[16..24].copy_from_slice(&dec.maxpx.to_le_bytes());
    db[24..32].copy_from_slice(&addr(&*mb).to_le_bytes());
    db[40..48].copy_from_slice(&dec.maxmem.to_le_bytes());
    let sf = Box::new([0u8; 32]);
    let (rax, fault) = pft_call(&s, s.dec, [data.as_ptr() as u64, data.len() as u64, addr(&*sf), addr(&*db)], FW_DECODE);
    if fault {
        return ERR_FAULT; // (a surface it made is dropped)
    }
    meta_from(&mb[..], meta);
    let rec = SurfAbi::from(&sf[..]);
    let r = rax as i32;
    if r != 0 {
        blk_free(rec.px);
        return r;
    }
    if dec.flags & img_core::IMGD_PROBE != 0 && rec.px == 0 {
        return 0; // IMGD_PROBE: no pixels needed
    }
    let maxd = img_core::IMG_MAXDIM as i32;
    if rec.px == 0
        || rec.w <= 0
        || rec.h <= 0
        || rec.w > maxd
        || rec.h > maxd
        || rec.stride < rec.w * 4
        || (rec.fmt != FMT_ARGB && rec.fmt != FMT_PARGB)
    {
        return ERR_FAIL; // (a bad surface is dropped, not freed)
    }
    let mut o = Surface::new(rec.w, rec.h, rec.fmt);
    let (ds, nb) = (o.stride as usize, rec.w as usize * 4);
    let Some(px) = plat::guard_read(rec.px as usize, rec.stride as usize * (rec.h as usize - 1) + nb) else { return ERR_FAIL };
    for y in 0..rec.h as usize {
        let so = y * rec.stride as usize;
        o.px[y * ds..y * ds + nb].copy_from_slice(&px[so..so + nb]);
    }
    blk_free(rec.px);
    *out = o;
    0
}

// pft_encode(sf, opt, out) -> 0 / ERR_* -- the plugin gets a straight FMT_ARGB surface; its
// buffer must be a PA_ALLOC block holding len bytes
fn pft_encode_slot(j: usize, sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    let Some(s) = pft_slot(j) else { return img_core::ERR_UNSUPPORTED };
    let conv;
    let src = if sf.fmt == FMT_ARGB {
        sf
    } else {
        match img_core::meta::surf_convert(sf, FMT_ARGB) {
            Ok(c) => {
                conv = c;
                &conv
            }
            Err(e) => return e,
        }
    };
    let rec = SurfAbi {
        px: src.px.as_ptr() as u64,
        w: src.w,
        h: src.h,
        stride: src.stride,
        fmt: FMT_ARGB,
        flags: 0,
        opac: 255,
        blend: 0,
        aux: 0,
    };
    let mut ob = Box::new([0u8; 64]);
    for (k, v) in [
        opt.fmt,
        opt.quality,
        opt.flags,
        opt.bitdepth,
        opt.samplebits,
        opt.dither,
        opt.athresh,
        opt.chroma,
        opt.effort,
        opt.ddsfmt,
        opt.mipfilter,
        opt.tiffcomp,
    ]
    .iter()
    .enumerate()
    {
        ob[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    let cells = Box::new([0u64; 2]); // buf, len
    let (rax, fault) = pft_call(&s, s.enc, [addr(&rec), addr(&*ob), addr(&cells[0]), addr(&cells[1])], FW_ENCODE);
    if fault {
        return ERR_FAULT;
    }
    let (b, len) = (cells[0], cells[1]);
    let r = rax as i32;
    if r != 0 {
        blk_free(b); // (img_encode frees *buf)
        return r;
    }
    // the buffer: a block of at least len bytes
    let ok = b != 0 && len != 0 && blk_size(b).is_some_and(|n| n as u64 >= len);
    if !ok {
        return ERR_FAIL;
    }
    let Some(d) = plat::guard_read(b as usize, len as usize) else { return ERR_FAIL };
    blk_free(b);
    *out = d;
    0
}

fn pft_probe<const J: usize>(data: &[u8], meta: &mut ImgMeta) -> bool {
    pft_probe_slot(J, data, meta)
}

fn pft_decode<const J: usize>(data: &[u8], out: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    pft_decode_slot(J, data, out, dec, meta)
}

fn pft_encode<const J: usize>(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    pft_encode_slot(J, sf, opt, out)
}

static PFT_PROBE: [img_core::ProbeFn; 8] = slot_table!(pft_probe; 0 1 2 3 4 5 6 7);
static PFT_DECODE: [img_core::DecodeFn; 8] = slot_table!(pft_decode; 0 1 2 3 4 5 6 7);
static PFT_ENCODE: [img_core::EncodeFn; 8] = slot_table!(pft_encode; 0 1 2 3 4 5 6 7);
const _: () = assert!(PFT_MAX == 8);

// ================================================================== loading

/// plug_init() -- see the file header
pub(crate) fn plug_init(p: &mut Pn, _ui: &mut Ui) {
    plat::guard_init();
    ::fx::use_all(); // built-ins first (settings keep their order)
    img_all::use_all_codecs();
    if let Some(dir) = p1_arg("--plugin-fuzz") {
        plug_fuzz(p, &dir);
        return;
    }
    if !plug_root(p) {
        return;
    }
    for (k, folder) in S_FOLDERS.iter().enumerate() {
        for name in plug_scan(p, folder) {
            plug_load(p, k, &name);
        }
    }
    if !p.plug.fxdesc.is_empty() {
        let g: &'static [FxDesc] = Vec::leak(core::mem::take(&mut p.plug.fxdesc));
        ::fx::register(g);
    }
}

// plug_root() -> true with the root folder set, false = no plugins (--no-plugins)
pub(crate) fn root(p: &Pn) -> &str { &p.plug.root }
fn plug_root(p: &mut Pn) -> bool {
    if plat::args().iter().skip(1).any(|a| a == "--no-plugins") {
        return false;
    }
    let root = match p1_arg("--plugins") {
        Some(v) if v.starts_with('@') => {
            // "@name": <exe folder>/name (build.py test)
            plug_exedir().map(|d| fmt_cap(format!("{}/{}", d, &v[1..]), PATH_CAP - 256))
        }
        Some(v) => Some(fmt_cap(v, PATH_CAP - 256)),
        None => match plat::getenv("PDN_PLUGIN_DIR") {
            Some(e) if !e.is_empty() => Some(fmt_cap(e, PATH_CAP - 256)),
            _ => plug_exedir(),
        },
    };
    match root {
        Some(root) => {
            p.plug.root = root;
            true
        }
        None => false,
    }
}

// p1_arg(name) -> value of "name=VALUE" or of "name VALUE" on the command line (the "=" form
// keeps fileio from taking the value for a file to open)
fn p1_arg(name: &str) -> Option<String> {
    for a in plat::args().iter().skip(1) {
        if let Some(rest) = a.strip_prefix(name) {
            if let Some(v) = rest.strip_prefix('=') {
                return Some(String::from(v));
            }
        }
    }
    plat::arg_value(name)
}

// plug_exedir() -- the folder of the running executable, never derived from argv[0] (a bare
// name would resolve plugins from the current directory); None = no default plugin root
fn plug_exedir() -> Option<String> {
    #[cfg(any(windows, target_os = "linux"))]
    let exe = plat::current_exe()?;
    #[cfg(not(any(windows, target_os = "linux")))]
    let exe = String::new(); // native plugins are not loadable there (plat::lib_open)
    let k = exe.rfind(['/', '\\'])?;
    Some(fmt_cap(String::from(&exe[..k]), PATH_CAP - 256))
}

// plug_scan(folder) -> the plugin files of <root>/folder, sorted
fn plug_scan(p: &Pn, folder: &str) -> Vec<String> {
    let dir = fmt_cap(format!("{}/{}", p.plug.root, folder), PATH_CAP);
    let mut names: Vec<String> = Vec::new();
    plat::dir_list(&dir, &mut |name, is_dir, _| {
        if is_dir || !name.ends_with(S_LIBEXT) || names.len() >= NAMES_MAX || name.len() >= FNAME_CAP - 1 {
            return;
        }
        names.push(String::from(name));
    });
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    names
}

// plug_load(folder, name) -- load one plugin file (errors -> its record)
fn plug_load(p: &mut Pn, folder: usize, name: &str) {
    let managed_path=format!("{}/{}/{}",p.plug.root,S_FOLDERS[folder],name);
    if crate::integrations::paint_dot_net::catalog::is_managed(&managed_path) {crate::integrations::paint_dot_net::catalog::managed_found(&managed_path);return;}
    let i = PLUGS.lock().len();
    if i >= PLUG_MAX {
        if !p.plug.full_logged {
            p.plug.full_logged = true;
            let s = format!("PaintCraft: zu viele Plugins (höchstens {}), der Rest wird nicht geladen\n", PLUG_MAX);
            plat::write_stderr(s.as_bytes());
        }
        return;
    }
    let file = fmt_cap(format!("{}/{}", S_FOLDERS[folder], name), PL_FILE_CAP);
    PLUGS.lock().push(PlugRec { folder: folder as u32, file, ..PlugRec::default() });
    PSTATE[i].store(PS_LOADING, SeqCst);
    let path = fmt_cap(format!("{}/{}/{}", p.plug.root, S_FOLDERS[folder], name), PATH_CAP);
    let tag = i as u64 + 1;
    // ---- its rights ("<name>.permissions" beside it; none: plat's default)
    let rights = plat::policy::plugin_rights(&path);
    {
        let mut r = RIGHTS.lock();
        r.resize(i + 1, None);
        r[i] = rights.as_ref().ok().cloned();
    }
    if let Err(e) = rights {
        plug_reject(i, format!("Die Rechte des Plugins sind fehlerhaft: {}", e));
        return;
    }
    // ---- the loader (guarded as well: it reads an untrusted file)
    let mut info = GuardInfo { tag, ..GuardInfo::default() };
    let lib = match plat::guard_run(|| plat::lib_open(&path), Some(&mut info)) {
        Ok(Ok(l)) => l,
        Ok(Err(e)) => {
            plug_reject(i, String::from(S_LIBE[if e as usize <= 7 { e as usize } else { 0 }]));
            return;
        }
        Err(_) if info.kind != 0 => {
            plug_fault_msg(&mut PLUGS.lock()[i], FW_LOADER, &info, 0, 0);
            PSTATE[i].store(PS_REJECTED, SeqCst);
            plug_log(i);
            return;
        }
        Err(_) => {
            plug_reject(i, String::from(S_LIBE[0]));
            return;
        }
    };
    let (base, span) = (lib.base() as u64, lib.span() as u64);
    // ---- pn_plugin_query
    let mut info = GuardInfo { tag, ..GuardInfo::default() };
    let q = plat::guard_run(|| plat::lib_sym(&lib, "pn_plugin_query"), Some(&mut info)).unwrap_or(0);
    if info.kind != 0 || q == 0 || plat::lib_seg(&lib, q).1 & plat::LIBA_X == 0 {
        plug_reject(i, String::from("Die Funktion pn_plugin_query fehlt (keine Plugin-Bibliothek)."));
        return;
    }
    let (inf, _, info) = gcall(q as u64, [addr(&PNAPI), 0, 0, 0], tag);
    if info.kind != 0 {
        plug_fault_msg(&mut PLUGS.lock()[i], FW_QUERY, &info, base, span);
        PSTATE[i].store(PS_CRASHED, SeqCst);
        plug_log(i);
        return;
    }
    if inf == 0 {
        plug_reject(i, String::from("Das Plugin hat das Laden abgelehnt (pn_plugin_query lieferte 0)."));
        return;
    }
    // ---- validation and registration
    match plug_import(p, i, &lib, inf) {
        Ok(()) => {
            PLUGS.lock()[i].lib = Some(lib);
            PSTATE[i].store(PS_LOADED, SeqCst);
        }
        Err(msg) => plug_reject(i, msg), // nothing of it is registered; the library is unloaded
    }
}

// ------------------------------------------------------------------ validation helpers

// chk_mem(h, p, len, liba) -> [p, p + len) lies inside one image segment with the flags liba
fn chk_mem(h: &plat::Lib, p: u64, len: usize, liba: u32) -> bool {
    if p == 0 {
        return false;
    }
    let (end, fl) = plat::lib_seg(h, p as usize);
    end != 0 && fl & liba == liba && len <= end - p as usize
}

// rd(h, p, len) -> the bytes of a readable piece of the image
fn rd(h: &plat::Lib, p: u64, len: usize) -> Option<Vec<u8>> {
    if !chk_mem(h, p, len, plat::LIBA_R) {
        return None;
    }
    plat::lib_read(h, p as usize, len)
}

// chk_str(h, p, max) -> a NUL-terminated string of 1..max bytes in a readable segment
// without control characters
fn chk_str(h: &plat::Lib, p: u64, max: usize) -> Option<String> {
    if p == 0 {
        return None;
    }
    let (end, fl) = plat::lib_seg(h, p as usize);
    if end == 0 || fl & plat::LIBA_R == 0 {
        return None;
    }
    let n = (end - p as usize).min(max + 1);
    let b = plat::lib_read(h, p as usize, n)?;
    let k = b.iter().position(|&c| c == 0)?;
    if k == 0 || b[..k].iter().any(|&c| c < 32 || c == 127) {
        return None;
    }
    Some(String::from_utf8_lossy(&b[..k]).into_owned())
}

// chk_fn(h, fn) -> fn lies in an executable segment
fn chk_fn(h: &plat::Lib, f: u64) -> bool {
    f != 0 && plat::lib_seg(h, f as usize).1 & plat::LIBA_X != 0
}

// key_ok(s) -> s can be an INI key / menu name: no '=' ';' '#' '[' or blank at the start, no
// blank at the end, no '='
fn key_ok(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || matches!(b[0], b' ' | b';' | b'#' | b'[') || b.contains(&b'=') {
        return false;
    }
    b[b.len() - 1] != b' '
}

// f64_ok(bits) -> the double is finite
fn f64_ok(bits: u64) -> bool {
    (bits >> 52) & 0x7FF != 0x7FF
}

/// what import stages before the commit
struct StagedFx {
    flags: u32,
    cat: u32,
    name: String,
    prep: u64,
    rend: u64,
    free: u64,
    props: Vec<StagedProp>,
}

struct StagedProp {
    typ: u8,
    dec: u8,
    slot: u8,
    name: String,
    min: f64,
    max: f64,
    def: f64,
    def_y: f64,
    choices: Vec<String>,
}

struct StagedFt {
    name: String,
    entry: String,
    mime: String,
    probe: u64,
    dec: u64,
    enc: u64,
    caps: u32,
    exts: Vec<String>,
}

// plug_import(i, lib, info) -> Ok registered / Err(message) -- validate PLUGININFO and its
// descriptors, then register
fn plug_import(p: &mut Pn, i: usize, h: &plat::Lib, inf: u64) -> Result<(), String> {
    // ---- the record itself
    let b = rd(h, inf, PI_SIZE_V1).ok_or(String::from(S_BADPTR))?;
    if le32(&b, 0) != PLUGIN_MAGIC {
        return Err(String::from("Ungültige Plugin-Info (Kennung)."));
    }
    let size = le32(&b, 4);
    if (size as usize) < PI_SIZE_V1 || size > 4096 {
        return Err(format!("Ungültige Plugin-Info (Größe {}).", size));
    }
    // ---- texts
    let name = chk_str(h, le64(&b, 16), NAME_MAX).ok_or(String::from("Ungültige Plugin-Info (Name)."))?;
    PLUGS.lock()[i].name = fmt_cap(name, 64);
    let ver = chk_str(h, le64(&b, 24), 31).ok_or(String::from("Ungültige Plugin-Info (Version)."))?;
    PLUGS.lock()[i].ver = fmt_cap(ver, 32);
    let apimin = le32(&b, 8);
    if apimin > PNAPI_VERSION {
        return Err(format!("Das Plugin benötigt die Plugin-API-Version {}, vorhanden ist {}.", apimin, PNAPI_VERSION));
    }
    if apimin == 0 {
        return Err(String::from("Ungültige Plugin-Info (API-Version 0)."));
    }
    if le64(&b, 32) != 0 {
        let a = chk_str(h, le64(&b, 32), NAME_MAX).ok_or(String::from("Ungültige Plugin-Info (Autor)."))?;
        PLUGS.lock()[i].author = fmt_cap(a, 64);
    }
    if le64(&b, 40) != 0 {
        let u = chk_str(h, le64(&b, 40), 127).ok_or(String::from("Ungültige Plugin-Info (Herkunft)."))?;
        PLUGS.lock()[i].url = fmt_cap(u, 128);
    }
    // ---- counts and capacity
    let (nfx, nft, nsh) = (le32(&b, 48), le32(&b, 52), le32(&b, 72));
    if nfx > PER_PLUG_FX || nft > PER_PLUG_FT || nsh > PER_PLUG_SH {
        return Err(String::from("Ungültige Plugin-Info (zu viele Einträge)."));
    }
    if p.plug.npfx + nfx as usize > PFX_MAX || p.plug.npft + nft as usize > PFT_MAX || p.plug.shapes.len() + nsh as usize > PSH_MAX {
        return Err(String::from("Kein Platz mehr für weitere Plugin-Effekte, -Dateitypen oder -Formen."));
    }
    if nfx | nft | nsh == 0 {
        return Err(String::from("Das Plugin enthält weder Effekte noch Dateitypen noch Formen."));
    }
    // ---- effects
    let mut sfx: Vec<StagedFx> = Vec::new();
    if nfx != 0 {
        let list = rd(h, le64(&b, 56), nfx as usize * 8).ok_or(String::from(S_BADPTR))?;
        for k in 0..nfx as usize {
            let f = import_fx(p, h, le64(&list, k * 8), &sfx)?;
            sfx.push(f);
        }
    }
    // ---- file types
    let mut sft: Vec<StagedFt> = Vec::new();
    if nft != 0 {
        let ft = le64(&b, 64);
        if !chk_mem(h, ft, nft as usize * FT_SIZE, plat::LIBA_R) {
            return Err(String::from(S_BADPTR));
        }
        for k in 0..nft as usize {
            let t = import_ft(p, h, ft + (k * FT_SIZE) as u64, &sft)?;
            sft.push(t);
        }
    }
    // ---- shapes
    let mut ssh: Vec<(String, String)> = Vec::new();
    if nsh != 0 {
        let sh = le64(&b, 80);
        if !chk_mem(h, sh, nsh as usize * PSD_SIZE, plat::LIBA_R) {
            return Err(String::from(S_BADPTR));
        }
        for k in 0..nsh as usize {
            let s = import_shape(p, h, sh + (k * PSD_SIZE) as u64, ssh.len())?;
            ssh.push(s);
        }
    }
    // ---- commit (nothing below can fail)
    PLUGS.lock()[i].nfx = nfx;
    for f in sfx {
        let k = p.plug.npfx;
        if k >= PFX_MAX {
            break;
        }
        let name: &'static str = String::leak(f.name);
        let props: Vec<FxProp> = f.props.into_iter().map(prop_static).collect();
        p.plug.fxdesc.push(FxDesc {
            id: PLUG_FXID0 + k as u32,
            flags: f.flags,
            name,
            prepare: if f.prep != 0 { Some(PFX_PREP[k]) } else { None },
            render: PFX_REND[k],
            props: Vec::leak(props),
            category: f.cat,
            user: i,
        });
        PFX.lock().push(PfxSlot { plug: i, prep: f.prep, rend: f.rend, free: f.free, name });
        p.plug.npfx += 1;
    }
    for t in sft {
        let j = p.plug.npft;
        if j >= PFT_MAX {
            break;
        }
        let mut caps = 0;
        if t.dec != 0 {
            caps |= img_core::IFC_DECODE;
        }
        if t.enc != 0 {
            caps |= img_core::IFC_ENCODE;
        }
        let c: &'static Codec = Box::leak(Box::new(Codec {
            probe: if t.probe != 0 { Some(PFT_PROBE[j]) } else { None },
            decode: if t.dec != 0 { Some(PFT_DECODE[j]) } else { None },
            encode: if t.enc != 0 { Some(PFT_ENCODE[j]) } else { None },
            caps,
            ..Codec::EMPTY
        }));
        PFT.lock().push(PftSlot { plug: i, probe: t.probe, dec: t.dec, enc: t.enc });
        let mime: &'static str = if t.mime.is_empty() { "" } else { String::leak(t.mime) };
        let fmt = img_core::io::register_dyn(String::leak(t.name), String::leak(t.entry), mime, t.caps, c);
        if fmt == 0 {
            PFT.lock().pop(); // no free format slot (cannot happen: checked)
            continue;
        }
        p.plug.npft += 1;
        PLUGS.lock()[i].nft += 1;
    }
    PLUGS.lock()[i].nsh = nsh;
    for (n, d) in ssh {
        p.plug.shapes.push((i, n, d));
    }
    Ok(())
}

// prop_static(prop) -> the FxProp of a staged property (its strings leaked: they live as long
// as the registered effect)
fn prop_static(s: StagedProp) -> FxProp {
    let choices: Vec<&'static str> = s.choices.into_iter().map(String::leak).map(|x| &*x).collect();
    FxProp {
        typ: s.typ,
        dec: s.dec,
        slot: s.slot,
        flags: 0,
        name: String::leak(s.name),
        min: s.min,
        max: s.max,
        def: s.def,
        def_y: s.def_y,
        choices: Vec::leak(choices),
    }
}

// import_fx(desc, staged) -> staged effect / message
fn import_fx(p: &Pn, h: &plat::Lib, d: u64, staged: &[StagedFx]) -> Result<StagedFx, String> {
    if p.plug.npfx + staged.len() >= PFX_MAX {
        return Err(String::from("Kein Platz mehr für weitere Plugin-Effekte."));
    }
    let b = rd(h, d, FXD_SIZE).ok_or(String::from(S_BADPTR))?;
    let flags = le32(&b, 4);
    if flags & !FXF_PLUGIN_OK != 0 {
        return Err(String::from("Effekt: unbekannte Flags."));
    }
    let cat = le32(&b, 56);
    if cat > ::fx::FXC_STYLIZE {
        return Err(String::from("Effekt: unbekannte Kategorie."));
    }
    // name (menu label, settings key)
    let name = chk_str(h, le64(&b, 8), FX_NAME_MAX).ok_or(String::from("Effekt: ungültiger Name."))?;
    if !key_ok(&name) {
        return Err(String::from("Effekt: ungültiger Name."));
    }
    if ::fx::find(&name).is_some() || staged.iter().any(|s| s.name == name) {
        return Err(format!("Effekt-Name bereits vergeben: {}", name));
    }
    // functions
    let (prep, rend, free) = (le64(&b, 16), le64(&b, 24), le64(&b, 32));
    if !chk_fn(h, rend) {
        return Err(String::from("Effekt: render fehlt oder liegt außerhalb des Codes."));
    }
    if prep != 0 && !chk_fn(h, prep) {
        return Err(String::from("Effekt: prepare liegt außerhalb des Codes."));
    }
    if free != 0 && !chk_fn(h, free) {
        return Err(String::from("Effekt: free liegt außerhalb des Codes."));
    }
    // properties
    let np = le32(&b, 40) as usize;
    if np > ::fx::FX_NSLOTS {
        return Err(String::from("Effekt: zu viele Eigenschaften."));
    }
    let mut props = Vec::new();
    if np != 0 {
        let pb = rd(h, le64(&b, 48), np * FXP_SIZE).ok_or(String::from(S_BADPTR))?;
        for k in 0..np {
            props.push(import_prop(h, &pb[k * FXP_SIZE..(k + 1) * FXP_SIZE]).ok_or(String::from("Effekt: ungültige Eigenschaft."))?);
        }
    }
    Ok(StagedFx { flags, cat, name, prep, rend, free, props })
}

// import_prop(FXPROP bytes) -> the checked property (name and choice list copied)
fn import_prop(h: &plat::Lib, b: &[u8]) -> Option<StagedProp> {
    let typ = b[0];
    if !(FP_INT..=FP_SEED).contains(&typ) {
        return None;
    }
    let slot = b[2];
    let last = if typ == FP_PAN { slot as usize + 1 } else { slot as usize };
    if last >= ::fx::FX_NSLOTS {
        return None;
    }
    let dec = b[1].min(8);
    let name = chk_str(h, le64(b, 8), NAME_MAX)?;
    let (minb, maxb, defb, chb) = (le64(b, 16), le64(b, 24), le64(b, 32), le64(b, 40));
    let (min, max, def) = (f64::from_bits(minb), f64::from_bits(maxb), f64::from_bits(defb));
    let mut s = StagedProp { typ, dec, slot, name, min, max, def, def_y: 0.0, choices: Vec::new() };
    if typ == FP_COLOR || typ == FP_SEED {
        return Some(s); // min / max unused, default = colour bits
    }
    if !f64_ok(minb) || !f64_ok(maxb) || !f64_ok(defb) || min > max {
        return None;
    }
    if typ == FP_PAN {
        // FP_PAN: y default as f64 bits
        if !f64_ok(chb) {
            return None;
        }
        s.def_y = f64::from_bits(chb);
        return Some(s);
    }
    if def < min || def > max {
        return None;
    }
    if typ != FP_CHOICE {
        return Some(s);
    }
    if min < 0.0 {
        return None;
    }
    let list = import_strlist(h, chb)?;
    if rt::fx::cvttd(max) as u32 >= list.len() as u32 {
        return None; // max choice index < entries
    }
    s.choices = list;
    Some(s)
}

// import_strlist(list) -> 1..CHOICE_MAX strings of 1..NAME_MAX bytes, then an empty string
fn import_strlist(h: &plat::Lib, mut a: u64) -> Option<Vec<String>> {
    let mut out = Vec::new();
    loop {
        let b = rd(h, a, 1)?;
        if b[0] == 0 {
            break;
        }
        if out.len() >= CHOICE_MAX {
            return None;
        }
        let s = chk_str(h, a, NAME_MAX)?;
        a += s.len() as u64 + 1;
        out.push(s);
    }
    if out.is_empty() { None } else { Some(out) }
}

// import_ft(ftdesc, staged) -> staged file type / message
fn import_ft(p: &Pn, h: &plat::Lib, d: u64, staged: &[StagedFt]) -> Result<StagedFt, String> {
    if p.plug.npft + staged.len() >= PFT_MAX {
        return Err(String::from("Kein Platz mehr für weitere Plugin-Dateitypen."));
    }
    let b = rd(h, d, FT_SIZE).ok_or(String::from(S_BADPTR))?;
    let name = chk_str(h, le64(&b, 0), 31).ok_or(String::from("Dateityp: ungültiger Name."))?;
    let mut mime = String::new();
    if le64(&b, 16) != 0 {
        mime = chk_str(h, le64(&b, 16), NAME_MAX).ok_or(String::from("Dateityp: ungültiger MIME-Typ."))?;
    }
    let exts = chk_str(h, le64(&b, 8), 127).ok_or(String::from("Dateityp: ungültige Dateiendungen."))?;
    let list = exts_check(&exts, staged)?;
    let lowered = list.join(";");
    let entry = format!("{}={}", lowered, name);
    // functions
    let (probe, dec, enc) = (le64(&b, 24), le64(&b, 32), le64(&b, 40));
    if dec | enc == 0 {
        return Err(String::from("Dateityp: weder Decoder noch Encoder."));
    }
    for f in [probe, dec, enc] {
        if f != 0 && !chk_fn(h, f) {
            return Err(String::from("Dateityp: Funktion liegt außerhalb des Codes."));
        }
    }
    Ok(StagedFt { name, entry, mime, probe, dec, enc, caps: le32(&b, 48) & IFC_PLUGIN_OK, exts: list })
}

// exts_check(exts, staged) -> the extensions in lower case / message -- "a;b;c": 1..8
// extensions of 1..15 characters [a-z0-9]; none may already be known (format_from_path) or
// staged
fn exts_check(exts: &str, staged: &[StagedFt]) -> Result<Vec<String>, String> {
    let bad = || String::from("Dateityp: ungültige Dateiendungen.");
    let mut out: Vec<String> = Vec::new();
    for e in exts.split(';') {
        let mut s = String::new();
        for c in e.bytes() {
            let c = c.to_ascii_lowercase();
            if !(c.is_ascii_lowercase() || c.is_ascii_digit()) || s.len() >= EXT_MAX {
                return Err(bad());
            }
            s.push(c as char);
        }
        if s.is_empty() {
            return Err(bad());
        }
        if out.len() >= 8 {
            return Err(bad());
        }
        let taken = img_core::io::format_from_path(&format!("x.{}", s)) != 0
            || staged.iter().any(|t| t.exts.iter().any(|x| *x == s));
        if taken {
            return Err(format!("Dateiendung bereits belegt: {}", s));
        }
        out.push(s);
    }
    Ok(out)
}

// import_shape(shapedesc, staged) -> (name, path data) / message
fn import_shape(p: &Pn, h: &plat::Lib, d: u64, staged: usize) -> Result<(String, String), String> {
    if p.plug.shapes.len() + staged >= PSH_MAX {
        return Err(String::from("Kein Platz mehr für weitere Plugin-Formen."));
    }
    let b = rd(h, d, PSD_SIZE).ok_or(String::from(S_BADPTR))?;
    let name = chk_str(h, le64(&b, 0), NAME_MAX).ok_or(String::from("Form: ungültiger Name."))?;
    let data = chk_str(h, le64(&b, 8), SHAPE_DATA_MAX).ok_or(String::from("Form: ungültige Pfaddaten."))?;
    // path data: letters, digits, blanks , . + -
    if !data.bytes().all(|c| matches!(c, b' ' | b',' | b'+' | b'-' | b'.') || c.is_ascii_alphanumeric()) {
        return Err(String::from("Form: ungültige Pfaddaten."));
    }
    Ok((name, data))
}

// ================================================================== shapes (shapes_lib.rs hook)

/// plug_shapes_feed() -- the shapes of every loaded plugin as one XAML document per plugin into
/// shl_add_xaml (called at the end of shl_load_dir, so they survive reloading the folder)
pub(crate) fn plug_shapes_feed(p: &mut Pn) {
    if p.plug.shapes.is_empty() {
        return;
    }
    let n = PLUGS.lock().len();
    for i in 0..n {
        let (loaded, nsh, file) = {
            let g = PLUGS.lock();
            (pstate(i) == PS_LOADED, g[i].nsh, g[i].file.clone())
        };
        if !loaded || nsh == 0 {
            continue;
        }
        let mut buf: Vec<u8> = Vec::new();
        for (_, name, data) in p.plug.shapes.iter().filter(|s| s.0 == i) {
            xaml_put(&mut buf, S_X_OPEN, false);
            xaml_put(&mut buf, name, true);
            xaml_put(&mut buf, S_X_GEO, false);
            xaml_put(&mut buf, data, true);
            xaml_put(&mut buf, S_X_CLOSE, false);
        }
        crate::tools::shapes_library::shl_add_xaml(p, &buf, &file);
    }
}

// xaml_put(buf, str, escape) -- append str (escape: & < > " as entities); the text is cut at
// XAML_CAP - 16 (the shape then fails to parse and is reported)
fn xaml_put(buf: &mut Vec<u8>, s: &str, escape: bool) {
    for c in s.bytes() {
        if buf.len() >= XAML_CAP - 16 {
            return;
        }
        let ent: &[u8] = match c {
            b'&' if escape => b"&amp;",
            b'<' if escape => b"&lt;",
            b'>' if escape => b"&gt;",
            b'"' if escape => b"&quot;",
            _ => {
                buf.push(c);
                continue;
            }
        };
        buf.extend_from_slice(ent);
    }
}

// ================================================================== fuzz run

// plug_fuzz(dir) -- --plugin-fuzz DIR: every file of DIR through lib_open / lib_sym /
// lib_close (guarded); counts -> stdout and #0x82402, every guarded fault (a loader bug) ->
// stdout with the file name
fn plug_fuzz(p: &mut Pn, dir: &str) {
    let root = fmt_cap(String::from(dir), PATH_CAP - 256);
    let mut names: Vec<String> = Vec::new();
    plat::dir_list(&root, &mut |name, is_dir, _| {
        if !is_dir {
            names.push(String::from(name));
        }
    });
    let fz = &mut p.plug.fz;
    for name in names {
        fz[0] += 1; // files
        let path = fmt_cap(format!("{}/{}", root, name), PATH_CAP);
        let mut info = GuardInfo::default();
        let r = plat::guard_run(|| plat::lib_open(&path), Some(&mut info));
        let fault = |fz: &mut [u32; 4], info: &GuardInfo| {
            fz[3] += 1; // a fault inside the loader: report it
            let s = format!(
                "plugin-fuzz: fault in {}: kind {} code 0x{:x} at 0x{:x}, address 0x{:x}\n",
                name, info.kind, info.code, info.pc, info.addr
            );
            plat::write_stdout(s.as_bytes());
        };
        match r {
            Ok(Ok(lib)) => {
                fz[1] += 1; // loaded
                let mut info = GuardInfo::default();
                let _ = plat::guard_run(|| plat::lib_sym(&lib, "pn_plugin_query"), Some(&mut info));
                if info.kind != 0 {
                    fault(fz, &info);
                }
                drop(lib);
            }
            Ok(Err(_)) => fz[2] += 1, // rejected
            Err(_) => fault(fz, &info),
        }
    }
    p.plug.fztxt = format!("{},{},{},{}", fz[0], fz[1], fz[2], fz[3]);
    plat::write_stdout(format!("plugin-fuzz: files,loaded,rejected,faults = {}\n", p.plug.fztxt).as_bytes());
}

// ================================================================== module contract

/// plug_states() -- "Plugin-Fehler…" visible while an error is listed
pub(crate) fn plug_states(p: &mut Pn, ui: &mut Ui) {
    let f = if plug_errors(&p.plug) != 0 { 0 } else { CF_HIDDEN };
    pn_cmd_set(ui, CMD_FX_PLUGIN_ERRORS, f);
}

// p1_sem(id, label, value) -- a hidden K_TEXT record
fn p1_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

/// plug_frame() -- probe records
pub(crate) fn plug_frame(p: &mut Pn, ui: &mut Ui) {
    let pl = &p.plug;
    let t = fmt_cap(
        format!("{},{},{},{},{}", plug_loaded(pl), plug_errors(pl), pl.npfx, pl.npft, pl.shapes.len()),
        96,
    );
    p1_sem(ui, ID_P1_PROBE, &t, plug_count(pl));
    p1_sem(ui, ID_P1_REC, &pl.rectxt, pl.recstate as i32);
    p1_sem(ui, ID_P1_FUZZ, &pl.fztxt, pl.fz[0] as i32);
    p1_sem(ui, ID_P1_FILTER, &pl.filtxt, 0);
}

/// plug_cmd(id, arg) -> eax 1 handled
pub(crate) fn plug_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_P1_PROBE => {
            let pl = &mut p.plug;
            pl.rectxt.clear();
            pl.recstate = 0;
            let g = PLUGS.lock();
            if let Some(r) = g.get(arg as usize) {
                let st = pstate(arg as usize);
                pl.recstate = st;
                pl.rectxt = fmt_cap(format!("{}|{}|{}|{}|{}|{}|{}", st, r.whr, r.fault.kind, r.nfx, r.nft, r.nsh, r.name), LINE_CAP);
            }
            drop(g);
            ui.repass();
            return true;
        }
        CMD_P1_FILTER => {
            // img_filter_list(arg = IFC_DECODE / IFC_ENCODE) with the NULs shown as '|'
            let mut l = img_core::io::filter_list(arg, LINE_CAP - 1);
            if !l.is_empty() {
                l.pop(); // up to the list's final NUL
            }
            for c in l.iter_mut() {
                if *c == 0 {
                    *c = b'|';
                }
            }
            p.plug.filtxt = String::from_utf8_lossy(&l).into_owned();
            ui.repass();
            return true;
        }
        CMD_FX_PLUGIN_ERRORS => {
            crate::workspace::dialogs::settings::set_cmd(p, ui, CMD_SET_PAGE, SP_PLUGINS as u32);
            return true;
        }
        _ => {}
    }
    if (CMD_P1_FIRST..=CMD_P1_LAST).contains(&id) {
        return true;
    }
    // a plugin menu row: refuse effects of a disabled plugin, else the effect runner
    let label = p.cmds.iter().find(|c| c.id == id).map(|c| c.label);
    if let Some(l) = label {
        let slot = PFX.lock().iter().find(|s| s.name == l).copied();
        if let Some(s) = slot {
            if pstate(s.plug) != PS_LOADED {
                crate::integrations::file_io::fio_error(
                    p,
                    Some(s.name),
                    "Das Plugin wurde nach einem Fehler deaktiviert (siehe Einstellungen ▸ Plugin-Fehler).",
                );
                return true;
            }
        }
    }
    crate::effects::fx_cmd(p, ui, id, arg)
}

// ================================================================== getters (settings page)

/// plug_count() -> plugin records
pub(crate) fn plug_count(_pl: &Plug) -> i32 {
    PLUGS.lock().len() as i32
}

/// plug_errors() -> records rejected or crashed
pub(crate) fn plug_errors(_pl: &Plug) -> i32 {
    let n = PLUGS.lock().len().min(PLUG_MAX);
    (0..n).filter(|&i| matches!(pstate(i), PS_REJECTED | PS_CRASHED)).count() as i32
}

/// plug_loaded() -> plugins in PS_LOADED
pub(crate) fn plug_loaded(_pl: &Plug) -> i32 {
    let n = PLUGS.lock().len().min(PLUG_MAX);
    (0..n).filter(|&i| pstate(i) == PS_LOADED).count() as i32
}

/// plug_text(i) -> (PS_* of record i (0 = none), one line for the settings page): loaded
/// "Name Version – Autor · Effects/x.dll: 2 Effekte/Korrekturen, 1 Dateitypen, 0 Formen",
/// errors "Effects/x.dll: message"
pub(crate) fn plug_text(_pl: &Plug, i: i32) -> (u32, String) {
    let g = PLUGS.lock();
    let Some(r) = g.get(i as u32 as usize) else { return (0, String::new()) };
    let st = pstate(i as usize);
    if st != PS_LOADED {
        return (st, format!("{}: {}", r.file, r.msg));
    }
    let author = if r.author.is_empty() { "unbekannter Autor" } else { r.author.as_str() };
    (
        st,
        format!(
            "{} {} – {} · {}: {} Effekte/Korrekturen, {} Dateitypen, {} Formen",
            r.name, r.ver, author, r.file, r.nfx, r.nft, r.nsh
        ),
    )
}
