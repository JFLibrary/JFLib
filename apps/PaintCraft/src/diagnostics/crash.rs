// crash -- PaintCraft: crash reports, like paint.net's CrashLogDialog (CORE-008 / -009 / -018).
// A crash (the library's crash handler calls crash_hook through plat_crash_hook) writes the
// protocol "Absturz_<Datum>_<Zeit>.txt" -- exception, address and offset in the program,
// registers, the return addresses found on the stack, then the diagnostics of the settings
// dialog -- into the crash folder: `--crash-dir DIR`, else CrashLogs next to settings.ini (no
// folder, no hook in test runs without a configuration).  With a real window PaintCraft is
// then started again with `--crash-report FILE` (and the same --config): that instance runs
// no other module and shows only the window "PaintCraft Absturz" -- the message, the protocol
// (Details anzeigen / ausblenden), In die Zwischenablage kopieren, Ordner der Absturzberichte
// öffnen, PaintCraft neu starten (a fresh instance) and Beenden.
//   crash_mode_init() -> eax 1 = report mode (main.s then skips every module)
//   crash_mode() -> eax 1 = report mode
//   crash_setup()       after set_init: the crash folder, plat_crash_hook
//   crash_states()      (Rust) the diagnostics snapshot of the hook (diag_states, every frame)
//   crash_frame()       the report window (report mode, instead of the main window content)
//   crash_cmd(id, arg) -> eax   report mode: UIC_CLOSE / UIC_EXIT end it
//   crash_test()        (cmd 911) a deliberate write to address 0x10 (tests)
// Test probe (report mode) ID_CRASH_FIRST + 20: value = CRF_* actions so far (headless: the
// buttons only record), text = the protocol file.
//                                              [apps/PaintCraft/src/crash.s]
//
// Rust model: the hook is a plain fn(&CrashInfo) that runs while Pn is borrowed by the code
// that crashed, so it cannot call into the app.  Everything it needs lives in one static
// (HOOK, fixed buffers: the crash path does not allocate for the protocol): the folder, the
// --config value, whether to spawn the report window, and a snapshot of the diagnostics text
// that crash_states refreshes every frame while the hook is installed (the asm builds the
// text in the hook).

use alloc::format;
use alloc::string::String;
use core::fmt::Write;

use rt::sync::SpinLock;
use ui::*;

use crate::app::constants::*;
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const PATH_CAP: usize = 1040;
const REPORT_CAP: usize = 65536; // protocol text (also the report window's buffer)
const DIAG_SNAP: usize = 8192; // the diagnostics snapshot (the settings text is <= 4 KB)
const KEEP_REPORTS: usize = 20; // protocols kept in the crash folder (older ones are removed at start)
const NAME_PREFIX: &str = "Absturz_";
const ID_CR_DETAILS: u32 = ID_CRASH_FIRST + 1;
const ID_CR_COPY: u32 = ID_CRASH_FIRST + 2;
const ID_CR_FOLDER: u32 = ID_CRASH_FIRST + 3;
const ID_CR_RESTART: u32 = ID_CRASH_FIRST + 4;
const ID_CR_QUIT: u32 = ID_CRASH_FIRST + 5;
const ID_CR_TEXT: u32 = ID_CRASH_FIRST + 6;
const ID_CR_PROBE: u32 = ID_CRASH_FIRST + 20;
const CRF_RESTART: i32 = 1; // done bits (probe)
const CRF_QUIT: i32 = 2;
const CRF_COPIED: i32 = 4;
const CRF_FOLDER: i32 = 8;
const WIN_W: i32 = 680; // report window (DIP)
const WIN_H: i32 = 520;
const S_EXE: &str = if cfg!(windows) { "PaintCraft.exe" } else { "PaintCraft" };
/// cr_where(ci, addr) -> (image, offset): the program, or (jxl-module) the loaded JPEG XL
/// writer module jxlenc.dll / jxlenc.so holding addr (offsets into its own .map)
fn cr_where(ci: &plat::CrashInfo, a: u64) -> (&'static str, u64) {
    match plat::module_at(a) {
        Some((name, base)) => (name, a.wrapping_sub(base)),
        None => (S_EXE, a.wrapping_sub(ci.base)),
    }
}

static KIND_NAMES: [&str; 7] =
    ["(unbekannt)", "Zugriffsverletzung", "Ungültiger Befehl", "Division durch null", "Busfehler", "Stapelüberlauf", "Ausnahme"];
static REG_NAMES: [&str; 17] =
    ["RAX", "RBX", "RCX", "RDX", "RSI", "RDI", "RBP", "RSP", "R8", "R9", "R10", "R11", "R12", "R13", "R14", "R15", "RIP"];

/// the state of crash.s (its g_* globals; the hook's part is in HOOK)
#[derive(Default)]
pub(crate) struct Crash {
    /// g_cr_mode: report mode
    mode: bool,
    placed: bool,
    details: bool,
    /// g_cr_done: CRF_* (probe)
    done: i32,
    /// g_cr_config: value of --config
    config: Option<String>,
    /// g_cr_path: the protocol (report mode)
    path: String,
    /// g_cr_dir: its folder
    dir: String,
    /// g_cr_buf: the protocol text (report mode)
    buf: String,
    /// the hook is installed (crash_states refreshes its diagnostics)
    hooked: bool,
}

// ------------------------------------------------------------------ report mode

/// crash_mode_init() -> eax 1 when `--crash-report FILE` asks for the report window: the file
/// is read now (a missing file shows an empty protocol)
pub(crate) fn crash_mode_init(p: &mut Pn, _ui: &mut Ui) -> bool {
    let c = &mut p.crash;
    c.mode = false;
    let Some(path) = plat::arg_value("--crash-report") else { return false };
    c.mode = true;
    c.path = cap_str(&path, PATH_CAP);
    if let Some(data) = crash_read(&c.path) {
        // str_copy: up to the first NUL, cut at REPORT_CAP - 1 bytes
        let n = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        let s = String::from_utf8_lossy(&data[..n]);
        c.buf = cap_str(&s, REPORT_CAP);
    }
    // the folder of the protocol (for "Ordner der Absturzberichte öffnen")
    c.dir = cut_name(&c.path);
    c.config = plat::arg_value("--config");
    true
}

// crash_read(path) -- only the first REPORT_CAP bytes: the rest would be cut anyway
fn crash_read(path: &str) -> Option<alloc::vec::Vec<u8>> {
    let f = plat::File::open(path, plat::FO_READ)?;
    let mut buf = alloc::vec![0u8; REPORT_CAP];
    let got = f.read_at(0, &mut buf)?;
    buf.truncate(got.min(REPORT_CAP));
    Some(buf)
}

/// crash_mode() -> eax 1 in report mode
pub(crate) fn crash_mode(c: &Crash) -> bool {
    c.mode
}

// cap_str(s, cap) -- str_copy: at most cap - 1 bytes (never inside a UTF-8 sequence)
fn cap_str(s: &str, cap: usize) -> String {
    fmt_cap(String::from(s), cap)
}

// cut_name(path) -- cut the last path component ("a/b/c.txt" -> "a/b"; no separator: ".")
fn cut_name(path: &str) -> String {
    match path.rfind(['/', '\\']) {
        Some(i) => String::from(&path[..i]),
        None => String::from("."),
    }
}

/// crash_cmd(id, arg) -> eax 1 handled -- report mode: closing the window; the placement timer
pub(crate) fn crash_cmd(_p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if id == UIC_TIMER {
        if arg != TMR_CRASH {
            return false;
        }
        cr_place(ui);
        return true;
    }
    if id == UIC_CLOSE {
        ui.quit();
        return true;
    }
    false
}

/// crash_frame() -- the window "PaintCraft Absturz" (the main window, made small and centred on
/// the first frame)
pub(crate) fn crash_frame(p: &mut Pn, ui: &mut Ui) {
    let c = &mut p.crash;
    if !c.placed {
        c.placed = true;
        ui.timer(TMR_CRASH, 1); // placed once the window runs (like settings.s)
        ui.set_title("PaintCraft Absturz");
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_BG, 0);
    ui.titlebar_begin("PaintCraft Absturz", 0);
    ui.titlebar_end();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | PAD_24, 12);
    ui.text(TS_SUBTITLE, crate::preferences::localization::tr("PaintCraft wurde unerwartet beendet"));
    ui.text(
        TS_BODY | TX_WRAP,
        "PaintCraft musste wegen eines unerwarteten Fehlers beendet werden. Was seit dem letzten Speichern geändert wurde, ist leider verloren.",
    );
    let line = fmt_cap(format!("Ein Absturzprotokoll wurde gespeichert: {}", c.path), PATH_CAP + 64);
    ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, &line);
    // Details anzeigen / ausblenden, In die Zwischenablage kopieren, Ordner öffnen
    ui.box_begin(0, BOX_H, 8);
    let lbl = if c.details { "Details ausblenden" } else { "Details anzeigen" };
    if ui.button(ID_CR_DETAILS, lbl) & EV_ACT != 0 {
        c.details = !c.details;
        ui.repass();
    }
    if ui.button(ID_CR_COPY, crate::preferences::localization::tr("In die Zwischenablage kopieren")) & EV_ACT != 0 {
        ui.clip_set(CLIP_TEXT, c.buf.as_bytes());
        c.done |= CRF_COPIED;
    }
    if ui.button(ID_CR_FOLDER, crate::preferences::localization::tr("Ordner der Absturzberichte öffnen")) & EV_ACT != 0 {
        ui.open_folder(&c.dir);
        c.done |= CRF_FOLDER;
    }
    ui.box_end();
    if c.details {
        ui.next_size(-1, -1);
        ui.textbox(ID_CR_TEXT, None, &mut c.buf, REPORT_CAP as u32 | TB_MULTI | TB_WRAP | TB_READONLY);
    } else {
        ui.spacer(-1);
    }
    // PaintCraft neu starten / Beenden
    ui.box_begin(0, BOX_H | BOX_END, 8);
    ui.next_flags(ST_ACCENT);
    if ui.button(ID_CR_RESTART, crate::preferences::localization::tr("PaintCraft neu starten")) & EV_ACT != 0 {
        cr_restart(c, ui);
    }
    if ui.button(ID_CR_QUIT, crate::preferences::localization::tr("Beenden")) & EV_ACT != 0 {
        c.done |= CRF_QUIT;
        if !ui.headless() {
            ui.quit(); // (tests: only recorded)
        }
    }
    ui.box_end();
    ui.box_end();
    ui.box_end();
    cr_probe(c, ui);
}

// cr_restart() -- "PaintCraft neu starten": a fresh PaintCraft (with the same --config), then quit
fn cr_restart(c: &mut Crash, ui: &mut Ui) {
    c.done |= CRF_RESTART;
    if ui.headless() {
        return; // tests: no process
    }
    match &c.config {
        Some(cfg) => plat::spawn_self(&["--config", cfg.as_str()]),
        None => plat::spawn_self(&[]),
    };
    ui.quit();
}

// cr_place() -- the main window at WIN_W x WIN_H DIP, centred in the work area
fn cr_place(ui: &mut Ui) {
    let mut r = WinRect::new(plat::PW_KEEP, plat::PW_KEEP, WIN_W, WIN_H, 0);
    if let Some(wa) = plat::win_work_area(0) {
        r.x = (wa[2].wrapping_sub(ui.dp(WIN_W)) >> 1).wrapping_add(wa[0]);
        r.y = (wa[3].wrapping_sub(ui.dp(WIN_H)) >> 1).wrapping_add(wa[1]);
    }
    ui.window_place(0, &r);
}

// cr_probe() -- ID_CR_PROBE: value = CRF_* done, text = the protocol path
fn cr_probe(c: &Crash, ui: &mut Ui) {
    ui.last_reset();
    ui.sem(K_TEXT, ID_CR_PROBE, Some(&c.path), c.done);
}

// ------------------------------------------------------------------ the crash handler side

/// fixed text buffer (no allocation in the crash handler)
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("")
    }
    fn set(&mut self, s: &str) {
        self.n = 0;
        let _ = self.write_str(s);
    }
}

impl<const N: usize> Write for Buf<N> {
    // fmt_buf semantics: at most N - 1 bytes in all, never cut inside a UTF-8 sequence
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let room = (N - 1).saturating_sub(self.n);
        let mut k = s.len().min(room);
        while k < s.len() && !s.is_char_boundary(k) {
            k -= 1;
        }
        self.b[self.n..self.n + k].copy_from_slice(&s.as_bytes()[..k]);
        self.n += k;
        Ok(())
    }
}

/// what the hook needs (g_cr_dir, g_cr_config, g_cr_spawn, g_cr_path, g_cr_buf, g_cr_name)
struct HookState {
    dir: Buf<PATH_CAP>,
    config: Buf<PATH_CAP>,
    has_config: bool,
    spawn: bool,
    path: Buf<PATH_CAP>,
    buf: Buf<REPORT_CAP>,
    diag: Buf<DIAG_SNAP>,
}

static HOOK: SpinLock<HookState> = SpinLock::new(HookState {
    dir: Buf::new(),
    config: Buf::new(),
    has_config: false,
    spawn: false,
    path: Buf::new(),
    buf: Buf::new(),
    diag: Buf::new(),
});

/// crash_setup() -- the crash folder (--crash-dir DIR, else CrashLogs next to settings.ini)
/// and the hook; nothing in a test run without either (the library's stderr line stays)
pub(crate) fn crash_setup(p: &mut Pn, ui: &mut Ui) {
    let config = plat::arg_value("--config");
    p.crash.config = config.clone();
    // a folder too long for the hook's buffers is not shortened (that would write elsewhere)
    let dir = if let Some(d) = plat::arg_value("--crash-dir").filter(|d| d.len() < PATH_CAP - 64) {
        d
    } else if p.st.on {
        let d = cut_name(&cap_str(&p.st.file, PATH_CAP - 64));
        cap_str(&format!("{}/CrashLogs", d), PATH_CAP - 64)
    } else {
        return;
    };
    {
        let mut h = HOOK.lock();
        h.dir.set(&dir);
        h.has_config = config.is_some();
        h.config.set(config.as_deref().unwrap_or(""));
        h.spawn = !ui.headless(); // only a real window starts the report window
    }
    cr_prune(&dir);
    p.crash.hooked = true;
    crash_states(p);
    plat::crash_hook(Some(crash_hook));
}

// cr_prune(dir) -- keep the newest KEEP_REPORTS - 1 protocols of the crash folder (the names
// begin with the date, so the name order is the age order); the next crash adds one
fn cr_prune(dir: &str) {
    let mut names: alloc::vec::Vec<String> = alloc::vec::Vec::new();
    plat::dir_list(dir, &mut |name: &str, is_dir: bool, _size: u32| {
        if !is_dir && name.starts_with(NAME_PREFIX) && name.ends_with(".txt") && names.len() < 4096 {
            names.push(String::from(name));
        }
    });
    if names.len() < KEEP_REPORTS {
        return;
    }
    names.sort();
    let old = names.len() - (KEEP_REPORTS - 1);
    for n in &names[..old] {
        plat::delete(&format!("{}/{}", dir, n));
    }
}

/// crash_states() -- (Rust) the hook's snapshot of the diagnostics text (set_diag_text); the
/// asm builds it in the hook, which Rust cannot (Pn is borrowed by the crashing code)
pub(crate) fn crash_states(p: &mut Pn) {
    if !p.crash.hooked {
        return;
    }
    let t = crate::workspace::dialogs::settings::set_diag_text(p);
    HOOK.lock().diag.set(&t);
}

/// crash_test() -- (cmd 911) an access violation for the tests: plat::crash_test writes to
/// address 0x10 (inlined: the call chain shows crash_test <- a1_diag_cmd as in the asm)
#[inline(always)]
pub(crate) fn crash_test() {
    plat::crash_test();
}

// crash_hook(info) -- (in the crash handler) the protocol, written twice: first the crash
// itself, then again with the diagnostics; with a real window PaintCraft is started again to
// show it
fn crash_hook(ci: &plat::CrashInfo) {
    // a crash while the state is being updated (or a second one at the same time) must not
    // wait for the lock forever
    let Some(mut g) = HOOK.try_lock() else { return };
    let h = &mut *g;
    h.buf.n = 0;
    let (dt, utc) = plat::date();
    // the file name: Absturz_JJJJ-MM-TT_hh-mm-ss.txt
    // (a second protocol within the same second gets a counter, none is overwritten)
    for seq in 0..100u32 {
        h.path.n = 0;
        let _ = write!(
            h.path,
            "{}/Absturz_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            h.dir.as_str(),
            dt.year,
            dt.month,
            dt.day,
            dt.hour,
            dt.min,
            dt.sec
        );
        if seq > 0 {
            let _ = write!(h.path, "_{}", seq);
        }
        let _ = h.path.write_str(".txt");
        if plat::file_exists(h.path.as_str()) == 0 {
            break;
        }
    }
    // the protocol
    let b = &mut h.buf;
    let _ = b.write_str("PaintCraft ist abgestürzt.\n\nPaintCraft hat diesen Bericht nach einem unerwarteten Fehler geschrieben. Er enthält Angaben, mit denen sich die Ursache finden und beheben lässt.\n\n");
    let _ = write!(b, "Programm: PaintCraft {}\n", crate::preferences::PN_VERSION);
    let _ = write!(b, "Zeitpunkt: {:04}-{:02}-{:02} {:02}:", dt.year, dt.month, dt.day, dt.hour);
    let _ = write!(b, "{:02}:{:02}{}\n", dt.min, dt.sec, if utc { " UTC" } else { "" });
    // the exception
    let kind = ci.kind.min(plat::CK_OTHER);
    let _ = write!(b, "Ausnahme: 0x{:X} {}", ci.code, KIND_NAMES[kind as usize]);
    if ci.kind == plat::CK_ACCESS || ci.kind == plat::CK_BUS {
        let how = match ci.access {
            0 => "beim Lesen an",
            1 => "beim Schreiben an",
            8 => "beim Ausführen an",
            _ => "an",
        };
        let _ = write!(b, " ({} Adresse 0x{:x})", how, ci.addr);
    }
    let _ = b.write_str("\n");
    if ci.kind == plat::CK_PANIC {
        let _ = write!(b, "Panic: {}\n", ci.msg); // (Rust only)
    }
    let (img, off) = cr_where(ci, ci.ip);
    let _ = write!(b, "Adresse: 0x{:x} ({} + 0x{:x}; die Funktionen stehen in der .map-Datei des Builds)\n", ci.ip, img, off);
    // registers, three per line
    let _ = b.write_str("\nRegister:\n");
    for (i, r) in ci.regs.iter().enumerate() {
        let _ = write!(b, "  {:<3} {:016x}", REG_NAMES[i], r);
        if (i + 1) % 3 == 0 || i + 1 == 17 {
            let _ = b.write_str("\n");
        }
    }
    let _ = b.write_str("\nAufrufkette (Rücksprungadressen im Programm, vom Stapel; die erste Zeile ist der Absturzort):\n");
    let _ = write!(b, "  {} + 0x{:x}\n", img, off);
    for &f in ci.frames.iter().take((ci.nframes as usize).min(ci.frames.len())) {
        let (img, off) = cr_where(ci, f);
        let _ = write!(b, "  {} + 0x{:x}\n", img, off);
    }
    plat::mkdir(h.dir.as_str());
    plat::write_file(h.path.as_str(), &h.buf.b[..h.buf.n]);
    // the diagnostics (the snapshot of crash_states): written again afterwards
    let _ = h.buf.write_str("\n---- Diagnose ----\n");
    let d = &h.diag;
    let _ = h.buf.write_str(d.as_str());
    plat::write_file(h.path.as_str(), &h.buf.b[..h.buf.n]);
    if !h.spawn {
        return;
    }
    // the report window: PaintCraft --crash-report FILE [--config DIR]
    if h.has_config {
        plat::spawn_self(&["--crash-report", h.path.as_str(), "--config", h.config.as_str()]);
    } else {
        plat::spawn_self(&["--crash-report", h.path.as_str()]);
    }
}
