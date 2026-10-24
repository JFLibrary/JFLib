// dlg_settings -- PaintCraft: "Einstellungen" (Alt+X, title bar button) like paint.net 5.1
// (agent S1, M7; docs/SETTINGS.md): a modal, resizable dialog window with the page list on
// the left, the page in a scroll view on the right and "Schließen" at the bottom.  Every
// change applies at once (like paint.net); closing the dialog (Schließen, Esc, close
// button) writes the settings file.  Loading / saving: settings.rs.
//                                              [apps/PaintCraft/src/dlg_settings.s]
//
//   set_init()                  st_init (settings file, factory values, early apply)
//   set_states()                st_states (late apply, main window tracking), CMD_SETTINGS
//   set_frame()                 the dialog window, test probes ID_SET_PROBE + 0..19
//   set_cmd(id, arg) -> bool    CMD_SETTINGS, CMD_SET_* (pn_set.inc), UIC_TIMER TMR_SET_*
//   set_diag_text() -> String   the diagnostics text, built now (crash.rs, dlg_misc.rs)
// Pages (SP_*): Benutzeroberfläche, Werkzeuge, Verlauf, Grafik, Farbmanagement,
// Aktualisierung (not available: no network / TLS), Diagnose, Plugin-Fehler,
// Stift & Tablett (PN1: Windows-Tinte = plat::pen_enable, detected devices).
// set_diag_text has no Ui (crash.rs calls it): the theme and dpi it reports are the ones of
// the last set_states / dialog frame.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::constants::*;
use crate::preferences::{st_hex, PN_VERSION};
use crate::app::formatting::fmt_cap;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const LBL_W: i32 = 200; // label column of a row (DIP)
const CTL_W: i32 = 280; // combo boxes / text boxes
const NUM_W: i32 = 120; // number boxes
const NAV_W: i32 = 220; // page list
const NAV_ROW: u32 = 36;
const DIAG_CAP: usize = 4096;
const LINE_CAP: usize = 256;
const SUM_N: usize = 5; // summary rows of the tool defaults
const PROBE_CAP: usize = 160;
const ACC_CAP: u32 = 16;
const CPU_CAP: usize = 64;
const FEAT_CAP: usize = 64;
const DIR_CAP: usize = 1040; // palette.rs PAL_PATH_CAP
const PKC_ERRORS: u32 = 0;
const PKC_LOADED: u32 = 1; // = plugins.rs PS_LOADED

static PAGE_NAMES: [&str; SP_COUNT as usize] = [
    "Benutzeroberfläche",
    "Werkzeuge",
    "Verlauf",
    "Grafik",
    "Farbmanagement",
    "Aktualisierung",
    "Diagnose",
    "Plugin-Fehler",
    "Stift & Tablett", // (PN1)
];
static PAGE_ICONS: [u32; SP_COUNT as usize] =
    [IC_VIEW, IC_BRUSH, IC_HISTORY, IC_PHOTO, IC_COLOR, IC_REFRESH, IC_REPAIR, IC_WARNING, IC_INKING_TOOL];
static FEAT_NAMES: [(u32, &str); 5] = [
    (plat::CPU_SSE41, ", SSE4.1"),
    (plat::CPU_AVX2, ", AVX2"),
    (plat::CPU_FMA, ", FMA"),
    (plat::CPU_POPCNT, ", POPCNT"),
    (plat::CPU_BMI2, ", BMI2"),
];
// ST_THEME (light, dark, system) -> combo row (Standard, Hell, Dunkel) and back
static THEME_TO_SEL: [i32; 3] = [1, 2, 0];
static SEL_TO_THEME: [i32; 3] = [THEME_SYSTEM, THEME_LIGHT, THEME_DARK];
const RANGE_BUDGET: Range = Range::new(0, BUDGET_MB_MAX, 256, 0);
const RANGE_THREADS: Range = Range::new(0, THREADS_MAX, 1, 0);
const S_OS: &str = if cfg!(windows) { "Windows (x64)" } else { "Linux (x86-64)" };
const S_MODULES: &str = if cfg!(windows) {
    "kernel32.dll, user32.dll, gdi32.dll (keine weiteren Bibliotheken)"
} else {
    "keine (statisches ELF, nur Systemaufrufe)"
};
// colour schemes (Standard = follow the system)
static THEME_LIST: [&str; 3] = ["Standard", "Hell", "Dunkel"];

// TOOL_* order (cmds.rs labels)
static TOOL_LIST: [&str; 19] = [
    "Rechteckige Auswahl",
    "Pixel verschieben",
    "Lasso-Auswahl",
    "Auswahl verschieben",
    "Elliptische Auswahl",
    "Zoomen",
    "Zauberstab",
    "Ansicht verschieben",
    "Farbeimer",
    "Farbverlauf",
    "Farbpinsel",
    "Radierer",
    "Zeichenstift",
    "Farbauswahl",
    "Klonen (Stempel)",
    "Neu färben",
    "Text",
    "Linie / Kurve",
    "Formen",
];
// tool option names (tool.rs), in TC_* value order
static AA_LIST: [&str; 2] = ["Kantenglättung aus", "Kantenglättung an"];
static SMOOTH_LIST: [&str; 2] = ["Ungeglätteter Weg", "Geglätteter Weg"];
static FLOOD_LIST: [&str; 2] = ["Zusammenhängend", "Global"];
static SMP_LIST: [&str; 2] = ["Ebene", "Bild"];
static SELMODE_LIST: [&str; 5] =
    ["Ersetzen", "Vereinigen", "Subtrahieren", "Schneiden", "Exklusiv (XOR)"];
static SELQ_LIST: [&str; 2] = ["Auswahl mit Kantenglättung", "Auswahl ohne Kantenglättung"];
static BLEND_LIST: [&str; 15] = [
    "Normal",
    "Multiplizieren",
    "Additiv",
    "Farbig nachbelichten",
    "Farbig abwedeln",
    "Reflektieren",
    "Leuchten",
    "Überlagerung",
    "Unterschied",
    "Negierung",
    "Aufhellen",
    "Verdunkeln",
    "Bildschirm",
    "XOR",
    "Überschreiben",
];

/// the state of dlg_settings.s (its g_* globals)
pub(crate) struct DSet {
    /// g_sd_open: dialog shown; g_sd_was: it was open (closing saves)
    open: Shared<i32>,
    was: bool,
    /// g_sd_page: SP_*
    page: i32,
    /// g_sd_themesel: combo row of the theme
    themesel: i32,
    /// g_sd_stat: sd_statics done
    stat: bool,
    acc: String,
    cpu: String,
    feat: String,
    sum: [String; SUM_N],
    tmpdef: String,
    paldir: String,
    diag: String,
    rect: Shared<WinRect>,
    /// the theme / dpi of the last frame (set_diag_text has no Ui)
    dark: bool,
    dpi: i32,
}

impl Default for DSet {
    fn default() -> DSet {
        DSet {
            open: Shared::new(0),
            was: false,
            page: 0,
            themesel: 0,
            stat: false,
            acc: String::new(),
            cpu: String::new(),
            feat: String::new(),
            sum: Default::default(),
            tmpdef: String::new(),
            paldir: String::new(),
            diag: String::new(),
            rect: Shared::new(WinRect::new(0, 0, 820, 560, 0)),
            dark: false,
            dpi: 96,
        }
    }
}

/// set_init() -- the settings file (settings.rs)
pub(crate) fn set_init(p: &mut Pn, ui: &mut Ui) {
    crate::preferences::st_init(p, ui);
}

/// set_states() -- late apply / main window tracking; Einstellungen is always enabled
pub(crate) fn set_states(p: &mut Pn, ui: &mut Ui) {
    crate::preferences::st_states(p, ui);
    pn_cmd_set(ui, CMD_SETTINGS, 0);
    sd_ui_values(p, ui);
}

// sd_ui_values() -- the theme / dpi the diagnostics report
fn sd_ui_values(p: &mut Pn, ui: &Ui) {
    p.dset.dark = ui.theme_dark();
    p.dset.dpi = ui.dpi();
}

/// set_cmd(id, arg) -> 1 handled
pub(crate) fn set_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        UIC_TIMER => return crate::preferences::st_timer(p, ui, arg),
        CMD_SETTINGS => sd_open(p, ui),
        CMD_SET_PAGE => {
            if arg < SP_COUNT as u32 {
                p.dset.page = arg as i32;
                sd_open(p, ui);
                sd_page_changed(p, ui);
            }
        }
        CMD_SET_SAVE => {
            crate::preferences::st_save(p, ui);
        }
        CMD_SET_RELOAD => {
            crate::preferences::st_reload(p, ui);
            sd_accent_text(p);
        }
        CMD_SET_FACTORY => {
            crate::preferences::st_factory(p, ui);
            sd_accent_text(p);
        }
        CMD_SET_FROMBAR => crate::preferences::st_tools_from_bar(p),
        CMD_SET_TRESET => crate::preferences::st_tools_reset(p),
        CMD_SET_DIAGCOPY => sd_diag_copy(p, ui),
        CMD_SET_SCREENS => crate::preferences::st_check_screens(p, ui),
        _ => return false,
    }
    ui.repass();
    true
}

// sd_open() -- show the dialog (its controls start from the current values)
fn sd_open(p: &mut Pn, ui: &mut Ui) {
    p.dset.open.set(1);
    p.dset.was = true;
    let mut t = p.st.st.theme;
    if t >= 3 {
        t = THEME_SYSTEM as u32;
    }
    p.dset.themesel = THEME_TO_SEL[t as usize];
    sd_accent_text(p);
    sd_ui_values(p, ui);
    sd_diag_build(p);
    ui.repass();
}

// sd_page_changed() -- a new page: scrolled to the top, fresh diagnostics
fn sd_page_changed(p: &mut Pn, ui: &mut Ui) {
    ui.scroll_to(ID_SET_PAGEBOX, 0, 0);
    if p.dset.page == SP_DIAG {
        sd_ui_values(p, ui);
        sd_diag_build(p);
    }
}

// ------------------------------------------------------------------ frame

/// set_frame() -- probes, the dialog window; a dialog closed since the last pass saves
pub(crate) fn set_frame(p: &mut Pn, ui: &mut Ui) {
    sd_probes(p, ui);
    if p.dset.was && p.dset.open.get() == 0 {
        p.dset.was = false;
        crate::preferences::st_save(p, ui);
    }
    let open = p.dset.open.clone();
    let rect = p.dset.rect.clone();
    if ui.window_begin(WINID_SETTINGS, crate::preferences::localization::tr("Einstellungen"), Some(&open), WF_DIALOG | WF_MODAL | WF_RESIZABLE, Some(&rect)) {
        sd_content(p, ui);
        ui.dialog_buttons(None, None, Some(crate::preferences::localization::tr("Schließen")));
    }
    ui.window_end();
}

// sd_content() -- page list | page
fn sd_content(p: &mut Pn, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 16);
    ui.next_size(NAV_W, -1);
    let mut sel = p.dset.page;
    let (first, end) = ui.list_begin(ID_SET_NAV, &mut sel, SP_COUNT, NAV_ROW);
    let mut r = first;
    while r < end && r < SP_COUNT {
        ui.list_item_begin(r);
        ui.icon(PAGE_ICONS[r as usize], 16);
        ui.text(TS_BODY, crate::preferences::localization::tr(PAGE_NAMES[r as usize]));
        ui.list_item_end();
        r += 1;
    }
    let (ev, s) = ui.list_end(&mut sel);
    p.dset.page = s;
    if ev & EV_ACT != 0 {
        sd_page_changed(p, ui);
    }
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_SET_PAGEBOX, SC_V);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | (4 << PAD_SHIFT), 10);
    if p.dset.page as u32 >= SP_COUNT as u32 {
        p.dset.page = 0;
    }
    ui.text(TS_SUBTITLE, crate::preferences::localization::tr(PAGE_NAMES[p.dset.page as usize]));
    match p.dset.page {
        0 => sd_page_ui(p, ui),
        1 => sd_page_tools(p, ui),
        2 => sd_page_hist(p, ui),
        3 => sd_page_gfx(p, ui),
        4 => sd_page_color(ui),
        5 => sd_page_updates(p, ui),
        6 => sd_page_diag(p, ui),
        7 => sd_page_plugins(p, ui),
        _ => sd_page_pen(p, ui),
    }
    ui.box_end();
    ui.scroll_end();
    ui.box_end();
}

// sd_row(label) -- open a row: label column, then the caller's control; close with box_end
fn sd_row(ui: &mut Ui, label: &str) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.next_size(LBL_W, 0);
    ui.text(TS_BODY, crate::preferences::localization::tr(label));
}

// sd_hrow() -- open a row of buttons (natural widths); close with box_end
fn sd_hrow(ui: &mut Ui) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H, 8);
}

// sd_note(str) -- a footnote (caption, secondary colour, wrapped)
fn sd_note(ui: &mut Ui, s: &str) {
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, s);
}

// sd_para(str) -- body text, wrapped
fn sd_para(ui: &mut Ui, s: &str) {
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, s);
}

// sd_nth(list, i) -> entry i ("" past the end)
fn sd_nth(list: &[&'static str], i: u32) -> &'static str {
    list.get(i as usize).copied().unwrap_or("")
}

// ------------------------------------------------------------------ pages

// Benutzeroberfläche: Farbschema, Akzentfarbe, Sprache, Animationen
fn sd_page_ui(p: &mut Pn, ui: &mut Ui) {
    sd_row(ui, crate::preferences::localization::tr("Farbschema:"));
    ui.next_size(CTL_W, 0);
    if ui.combobox(ID_SET_THEME, &mut p.dset.themesel, &crate::preferences::localization::list(&THEME_LIST)) & EV_ACT != 0 {
        let s = p.dset.themesel;
        if (s as u32) < 3 {
            let t = SEL_TO_THEME[s as usize] as u32;
            p.st.st.theme = t;
            ui.theme_set(t);
        }
    }
    ui.box_end();
    sd_note(ui, crate::preferences::localization::tr("Standard folgt der Einstellung des Betriebssystems (heller oder dunkler App-Modus)."));
    sd_row(ui, crate::preferences::localization::tr("Akzentfarbe:"));
    ui.next_size(CTL_W, 0);
    let ev = ui.textbox(ID_SET_ACCENT, Some("Standard"), &mut p.dset.acc, ACC_CAP);
    if ev & (EV_ACT | EV_COMMIT) != 0 {
        sd_accent_commit(p, ui, ev & EV_COMMIT != 0);
    }
    ui.box_end();
    sd_note(ui, crate::preferences::localization::tr("Sechs Hexadezimalziffern (RRGGBB), leer = Standardakzentfarbe."));
    sd_row(ui, crate::preferences::localization::tr("Sprache:"));
    ui.next_size(CTL_W, 0);
    let mut language = p.st.st.language as i32;
    if ui.combobox(ID_SET_LANG, &mut language, &crate::preferences::localization::NAMES) & EV_ACT != 0 { crate::preferences::localization::apply(p, ui, language as u32); }
    ui.box_end();
    sd_note(ui, crate::preferences::localization::tr("Weitere Übersetzungen folgen; nicht übersetzte Texte erscheinen auf Deutsch."));
    let mut a = p.st.st.anim as i32;
    let ev = ui.checkbox(ID_SET_ANIM, crate::preferences::localization::tr("Animationen"), &mut a);
    p.st.st.anim = a as u32;
    if ev & EV_ACT != 0 {
        ui.set_anim_off(p.st.st.anim == 0);
    }
    ui.text(TS_BODY_STRONG,crate::preferences::localization::tr("Arbeitsfläche"));
    for (id,label,color) in [(0x90500,"Hintergrundfarbe",&mut p.st.st.workspace),(0x90501,"Schachbrett hell",&mut p.st.st.checker0),(0x90502,"Schachbrett dunkel",&mut p.st.st.checker1)] {
        ui.dropdown_button(id,label);
        if ui.flyout_begin(id,FL_BELOW) {
            if ui.color_picker(id+10,color) & EV_ACT != 0 {*color |= 0xFF000000;ui.invalidate();}
        }
        ui.flyout_end();
    }
    let mut shadow=p.st.st.canvas_shadow as i32;
    if ui.checkbox(0x90503,crate::preferences::localization::tr("Schatten um das Bild"),&mut shadow) & EV_ACT != 0 {p.st.st.canvas_shadow=shadow as u32;ui.invalidate();}
    if ui.button(0x90504,crate::preferences::localization::tr("Arbeitsfläche zurücksetzen")) & EV_ACT != 0 {p.st.st.workspace=0;p.st.st.checker0=0xFFFFFFFF;p.st.st.checker1=0xFFCCCCCC;p.st.st.canvas_shadow=1;ui.invalidate();}
}

// sd_accent_text() -- the text box shows ST_ACCENT ("" = the default)
fn sd_accent_text(p: &mut Pn) {
    let a = p.st.st.accent;
    p.dset.acc = if a == 0 { String::new() } else { format!("{:06X}", a) };
}

// sd_accent_commit(commit) -- typed accent colour: empty or valid hex -> applied at once; on
// commit (focus lost / Enter) an invalid text shows the current colour again
fn sd_accent_commit(p: &mut Pn, ui: &mut Ui, commit: bool) {
    let v = if p.dset.acc.is_empty() { Some(0) } else { st_hex(&p.dset.acc).map(|v| v & 0xFF_FFFF) };
    if let Some(v) = v {
        if v != p.st.st.accent {
            p.st.st.accent = v;
            ui.accent_set(v);
            ui.invalidate();
        }
    }
    if commit {
        sd_accent_text(p);
        ui.repass();
    }
}

// Werkzeuge: default tool, summary of the default tool bar options, two buttons
fn sd_page_tools(p: &mut Pn, ui: &mut Ui) {
    sd_para(ui, crate::preferences::localization::tr("Hier legen Sie fest, mit welchem Werkzeug PaintCraft startet und welche Werte die Werkzeugleiste anfangs zeigt."));
    sd_row(ui, crate::preferences::localization::tr("Standardtool:"));
    ui.next_size(CTL_W, 0);
    let mut t = p.st.st.deftool as i32;
    ui.combobox(ID_SET_DEFTOOL, &mut t, &crate::preferences::localization::list(&TOOL_LIST));
    p.st.st.deftool = t as u32;
    ui.box_end();
    ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Standardeinstellungen der Symbolleiste"));
    sd_tool_summary(p);
    for k in 0..SUM_N {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, &p.dset.sum[k]);
    }
    sd_hrow(ui);
    if ui.button(ID_SET_FROMBAR, crate::preferences::localization::tr("Aktuelle Werkzeugleiste übernehmen")) & EV_ACT != 0 {
        crate::preferences::st_tools_from_bar(p);
    }
    if ui.button(ID_SET_TRESET, crate::preferences::localization::tr("Zurücksetzen")) & EV_ACT != 0 {
        crate::preferences::st_tools_reset(p);
    }
    ui.box_end();
}

// sd_tool_summary() -- sum rows from the stored defaults (ST_TOOLDEF)
fn sd_tool_summary(p: &mut Pn) {
    let tc = p.st.st.tooldef;
    // German decimal comma
    let w = fmt_cap(base::num::f64_format(tc.width), 32).replace('.', ",");
    let fill = if tc.fill == 0 { String::from("Volltonfarbe") } else { format!("Muster {}", tc.fill as i32) };
    let s = &mut p.dset.sum;
    s[0] = fmt_cap(
        format!("Pinseln und füllen: Pinselgröße {}, Härte {} %, Abstand {} %, {}", w, tc.hard as i32, tc.spacing as i32, fill),
        LINE_CAP,
    );
    s[1] = fmt_cap(
        format!(
            "Optionen: {}, Mischmodus {}, {}",
            sd_nth(&AA_LIST, tc.aa),
            sd_nth(&BLEND_LIST, tc.blend),
            sd_nth(&SMOOTH_LIST, tc.smooth)
        ),
        LINE_CAP,
    );
    s[2] = fmt_cap(
        format!("Farbeimer, Zauberstab und Neu färben: Toleranz {} %, Füllmodus {}", tc.tol as i32, sd_nth(&FLOOD_LIST, tc.flood)),
        LINE_CAP,
    );
    s[3] = fmt_cap(format!("Probennahme: {}", sd_nth(&SMP_LIST, tc.sample)), LINE_CAP);
    s[4] = fmt_cap(format!("Auswahl: {}, {}", sd_nth(&SELMODE_LIST, tc.selmode), sd_nth(&SELQ_LIST, tc.selqual)), LINE_CAP);
}

// Verlauf: memory budget, memory in use, spill folder (HIST-012..014)
fn sd_page_hist(p: &mut Pn, ui: &mut Ui) {
    sd_para(ui, crate::preferences::localization::tr("Die Rückgängig-Schritte aller geöffneten Bilder teilen sich ein Speicherbudget. Wird es überschritten, werden ältere Schritte komprimiert, in eine Auslagerungsdatei verschoben und zuletzt verworfen."));
    sd_row(ui, crate::preferences::localization::tr("Speicherbudget (MB):"));
    ui.next_size(NUM_W, 0);
    let mut b = p.st.st.budget_mb as i32;
    let ev = ui.number_box(ID_SET_BUDGET, &mut b, &RANGE_BUDGET);
    p.st.st.budget_mb = b as u32;
    if ev & (EV_ACT | EV_COMMIT) != 0 {
        crate::preferences::st_budget_apply(p);
    }
    ui.box_end();
    let l = fmt_cap(
        format!("0 = automatisch ({} MB: 40 % des Arbeitsspeichers, höchstens 4 GB).", p.st.bauto >> 20),
        LINE_CAP,
    );
    sd_note(ui, &l);
    let ram = crate::document::history::memory::hist_mem_ram(&p.hm) >> 20;
    let disk = crate::document::history::memory::hist_mem_disk(&p.hm) >> 20;
    let l = fmt_cap(format!("Belegt: {} MB im Arbeitsspeicher, {} MB in der Auslagerungsdatei.", ram, disk), LINE_CAP);
    sd_note(ui, &l);
    sd_row(ui, crate::preferences::localization::tr("Ordner der Auslagerungsdatei:"));
    ui.next_size(-1, 0);
    let ph = p.dset.tmpdef.clone();
    let ev = ui.textbox(ID_SET_TEMPDIR, Some(&ph), &mut p.st.st.tempdir, ST_PATH_CAP as u32);
    if ev & (EV_ACT | EV_COMMIT) != 0 {
        crate::preferences::st_tempdir_apply(p);
    }
    ui.box_end();
    sd_note(ui, crate::preferences::localization::tr("Leer = Temp-Ordner des Systems. Ein anderer Ordner gilt, sobald die Auslagerungsdatei angelegt wird (spätestens nach einem Neustart)."));
}

// Grafik: rendering device (CPU), instruction sets, worker threads
fn sd_page_gfx(p: &mut Pn, ui: &mut Ui) {
    sd_row(ui, crate::preferences::localization::tr("Rendering-Gerät:"));
    let l = fmt_cap(format!("CPU – {}", p.dset.cpu), LINE_CAP);
    ui.text(TS_BODY, &l);
    ui.box_end();
    sd_note(ui, crate::preferences::localization::tr("PaintCraft berechnet Werkzeuge, Korrekturen und Effekte auf der CPU (SIMD-Kernel auf mehreren Threads); eine GPU wird nicht verwendet."));
    sd_row(ui, crate::preferences::localization::tr("Befehlssätze:"));
    ui.text(TS_BODY, &p.dset.feat);
    ui.box_end();
    sd_note(ui, crate::preferences::localization::tr("Die Kernel werden beim Start nach dem Prozessor gewählt; --simd sse2 auf der Kommandozeile erzwingt die Basis-Kernel."));
    sd_row(ui, crate::preferences::localization::tr("Worker-Threads:"));
    ui.next_size(NUM_W, 0);
    let mut t = p.st.st.threads as i32;
    ui.number_box(ID_SET_THREADS, &mut t, &RANGE_THREADS);
    p.st.st.threads = t as u32;
    ui.box_end();
    let l = fmt_cap(
        format!(
            "0 = automatisch ({} logische Prozessoren). Aktuell {} Threads; Änderungen werden nach einem Neustart wirksam.",
            plat::cpu_count() as i32,
            plat::thr_count() as i32
        ),
        LINE_CAP,
    );
    sd_note(ui, &l);
}

// Farbmanagement: status only
fn sd_page_color(ui: &mut Ui) {
    ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Status"));
    sd_para(ui, crate::preferences::localization::tr("Die Farbdarstellung von PaintCraft erfolgt in sRGB."));
    sd_note(ui, crate::preferences::localization::tr("Erweiterte Farben von Windows (HDR, großer Farbraum) nutzt PaintCraft nicht; die Anzeige erfolgt immer in sRGB. Farbprofile von Bildern: Bild > Farbprofil…"));
}

// HTTPS release lookup through the optional Python 3 system runtime.
fn sd_page_updates(p: &mut Pn, ui: &mut Ui) {
    let mut automatic = p.st.st.update_auto as i32;
    let mut beta = p.st.st.update_beta as i32;
    ui.checkbox(ID_SET_UPD_AUTO, crate::preferences::localization::tr("Beim Programmstart prüfen, ob es eine neuere Version gibt"), &mut automatic);
    ui.checkbox(ID_SET_UPD_BETA, crate::preferences::localization::tr("Auch Testversionen (Beta) von PaintCraft anbieten"), &mut beta);
    p.st.st.update_auto = automatic as u32;
    p.st.st.update_beta = beta as u32;
    sd_hrow(ui);
    if ui.button(ID_SET_UPD_NOW, crate::preferences::localization::tr("Jetzt prüfen")) & EV_ACT != 0 {crate::integrations::services::check_updates(p,ui);}
    if ui.button(5595, crate::preferences::localization::tr("Veröffentlichungen öffnen")) & EV_ACT != 0 {ui.open_url("https://github.com/JFLibrary/JFLib/releases");}
    ui.box_end();
    sd_note(ui, crate::preferences::localization::tr("Die Prüfung benötigt Python 3 und fragt GitHub über HTTPS nach PaintCraft-Veröffentlichungen ab. Automatische Prüfung ist standardmäßig aus. Installation erfolgt von Hand."));
    ui.text(TS_BODY|TX_WRAP, &p.services.update_status);
}

// (PN1) Stift & Tablett: Windows-Tinte (pen input on / off, ST_PEN) and the devices found
fn sd_page_pen(p: &mut Pn, ui: &mut Ui) {
    let mut v = p.st.st.pen as i32;
    let ev = ui.checkbox(ID_SET_PEN, crate::preferences::localization::tr("Windows-Tinte"), &mut v);
    p.st.st.pen = v as u32;
    if ev & EV_ACT != 0 {
        plat::pen_enable(p.st.st.pen != 0);
    }
    sd_note(ui, crate::preferences::localization::tr("Damit werten Stifte und Grafiktabletts den Andruck aus und werden mit hoher Rate abgetastet. Macht ein Gerät Probleme, hilft es oft, die Option auszuschalten."));
    let n = plat::pen_devices();
    let t = if n > 0 {
        fmt_cap(format!("Gefundene Stifte und Grafiktabletts: {}", n), LINE_CAP)
    } else {
        String::from("Kein Stift und kein Grafiktablett gefunden.")
    };
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, &t);
    sd_sem(ui, ID_SET_PENDEV, &t, n); // probe: the text, value = the count
}

// Diagnose: read-only text + copy button
fn sd_page_diag(p: &mut Pn, ui: &mut Ui) {
    sd_para(ui, crate::preferences::localization::tr("Unten stehen die Angaben zur Diagnose."));
    ui.next_size(-1, 300);
    ui.textbox(ID_SET_DIAG, None, &mut p.dset.diag, DIAG_CAP as u32 | TB_MULTI | TB_WRAP | TB_READONLY);
    sd_hrow(ui);
    if ui.button(ID_SET_DIAGCOPY, crate::preferences::localization::tr("In die Zwischenablage kopieren")) & EV_ACT != 0 {
        sd_diag_copy(p, ui);
    }
    ui.box_end();
}

// ---- P1 (plugins.rs): Plugin-Fehler -- the error list and the loaded plugins (plug_text)
fn sd_page_plugins(p: &mut Pn, ui: &mut Ui) {
    if !plat::capabilities().supports(plat::CAP_NATIVE_PLUGINS) {
        sd_para(ui, crate::preferences::localization::tr("Native DLL-/SO-Plugins sind im Browser nicht verfügbar."));
        sd_note(ui, crate::preferences::localization::tr("Integrierte Effekte, Dateiformate und Formen sind weiterhin verfügbar."));
        return;
    }
    if crate::integrations::plugins::plug_errors(&p.plug) == 0 {
        sd_para(ui, crate::preferences::localization::tr("Alle Plugins wurden fehlerfrei geladen."));
    } else {
        sd_para(ui, crate::preferences::localization::tr("Diese Plugins ließen sich nicht laden oder wurden nach einem Fehler abgeschaltet:"));
        sd_plug_card(p, ui, PKC_ERRORS);
    }
    sd_plug_card(p, ui, PKC_LOADED);
    sd_para(ui, &crate::integrations::paint_dot_net::catalog::summary());
    for message in crate::integrations::paint_dot_net::catalog::messages() {sd_para(ui,&message);}
    sd_note(ui, crate::preferences::localization::tr("Plugins werden beim Start aus den Ordnern Effects, FileTypes und Shapes neben PaintCraft geladen (Windows: *.dll, Linux: *.so)."));
    sd_note(ui, crate::preferences::localization::tr("Originale Paint.NET-DLLs werden unter Windows zusätzlich aus Dokumente/PaintCraft App Files, Dokumente/Paint.NET App Files und der Paint.NET-Installation erkannt. Nach dem Kopieren neu starten. ZIP-Dateien zuerst entpacken; Abhängigkeiten neben der DLL belassen. Moderne Bitmap-/GPU-Plugins werden als noch nicht unterstützt angezeigt."));
}

// sd_plug_card(which) -- a card with the plug_text lines of the loaded plugins (PKC_LOADED) or
// of the plugins with an error (PKC_ERRORS)
fn sd_plug_card(p: &mut Pn, ui: &mut Ui, which: u32) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 8);
    if which == PKC_LOADED {
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Geladene Plugins"));
    }
    let mut shown = 0;
    let mut i = 0;
    while i < crate::integrations::plugins::plug_count(&p.plug) {
        let (st, line) = crate::integrations::plugins::plug_text(&p.plug, i);
        i += 1;
        if st == 0 {
            continue;
        }
        if (which == PKC_LOADED) != (st == PKC_LOADED) {
            continue;
        }
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, &fmt_cap(line, LINE_CAP));
        shown += 1;
    }
    if shown == 0 {
        ui.text(TS_BODY | TX_SECONDARY, crate::preferences::localization::tr("(keine)"));
    }
    ui.box_end();
}
// ---- end P1

// ------------------------------------------------------------------ diagnostics

// sd_statics() -- CPU name, instruction sets, temp folder, palette folder (once)
fn sd_statics(p: &mut Pn) {
    if p.dset.stat {
        return;
    }
    p.dset.stat = true;
    let b = plat::cpu_brand();
    p.dset.cpu = if b.is_empty() { String::from("unbekannt") } else { fmt_cap(b, CPU_CAP) };
    let f = plat::cpu_features();
    let mut s = String::from(if cfg!(target_arch = "wasm32") { "Skalar (WebAssembly)" } else { "SSE2" });
    for &(bit, name) in FEAT_NAMES.iter() {
        if f & bit != 0 {
            s.push_str(name);
        }
    }
    p.dset.feat = fmt_cap(s, FEAT_CAP);
    p.dset.tmpdef = match plat::temp_dir() {
        Some(t) if !t.is_empty() && t.len() < DIR_CAP => t,
        _ => String::new(),
    };
    p.dset.paldir = crate::workspace::palette::pal_folder(&p.pal).1;
}

// sd_diag_build() -- diag: version, OS, CPU, threads, memory, display, paths, modules
fn sd_diag_build(p: &mut Pn) {
    sd_statics(p);
    let d = &p.dset;
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("PaintCraft {} (JFLib, Rust)\n", PN_VERSION));
    lines.push(format!("Betriebssystem: {}\n", S_OS));
    lines.push(format!("Prozessor: {}\n", d.cpu));
    lines.push(format!("Befehlssätze: {}\n", d.feat));
    lines.push(format!("Logische Prozessoren: {}, Worker-Threads: {}\n", plat::cpu_count() as i32, plat::thr_count() as i32));
    lines.push(format!("Arbeitsspeicher: {} MB\n", plat::mem_total() >> 20));
    lines.push(format!(
        "Verlauf: Budget {} MB, im Arbeitsspeicher {} MB, ausgelagert {} MB\n",
        crate::document::history::memory::hist_budget(&p.hm) >> 20,
        crate::document::history::memory::hist_mem_ram(&p.hm) >> 20,
        crate::document::history::memory::hist_mem_disk(&p.hm) >> 20
    ));
    let dpi = d.dpi;
    lines.push(format!(
        "Anzeige: {} dpi ({} %), Farbschema {}\n",
        dpi,
        ((dpi as u32).wrapping_mul(100) / 96) as i32,
        if d.dark { "Dunkel" } else { "Hell" }
    ));
    lines.push(format!("Geöffnete Bilder: {}\n", crate::document::model::doc_count(&p.doc)));
    let file = if p.st.on && p.st.unreadable {
        format!("{} (nicht lesbar, wird nicht \u{fc}berschrieben)", p.st.file)
    } else if p.st.on {
        p.st.file.clone()
    } else { String::from("(keine: Testlauf ohne --config / PaintCraft_CONFIG_DIR)") };
    lines.push(format!("Einstellungsdatei: {}\n", file));
    let tmp = if p.st.st.tempdir.is_empty() { d.tmpdef.clone() } else { p.st.st.tempdir.clone() };
    lines.push(format!("Temp-Ordner: {}\n", tmp));
    lines.push(format!("Palettenordner: {}\n", d.paldir));
    lines.push(format!("Geladene Module: {}\n", S_MODULES));
    lines.push(format!("Effekte und Anpassungen: {}\n", fx_count()));
    lines.push(format!("Plugins: {} geladen\n", crate::integrations::plugins::plug_loaded(&p.plug))); // P1
    // sd_dcat: append while there is room (each piece cut to the room left)
    let mut out = String::new();
    for l in lines {
        if out.len() >= DIAG_CAP - 1 {
            break;
        }
        let room = DIAG_CAP - out.len();
        out.push_str(&fmt_cap(l, room));
    }
    p.dset.diag = out;
}

// fx_count() -- effects and adjustments registered
fn fx_count() -> i32 {
    ::fx::count() as i32 // pn-sys
}

/// set_diag_text() -> the diagnostics text, built now (crash.rs, dlg_misc.rs)
pub(crate) fn set_diag_text(p: &mut Pn) -> String {
    sd_diag_build(p);
    p.dset.diag.clone()
}

// sd_diag_copy() -- the diagnostics text into the clipboard
fn sd_diag_copy(p: &mut Pn, ui: &mut Ui) {
    sd_ui_values(p, ui);
    sd_diag_build(p);
    ui.clip_set(CLIP_TEXT, p.dset.diag.as_bytes());
}

// ------------------------------------------------------------------ test probes

// sd_sem(id, label, value) -- a hidden K_TEXT record (empty rectangle)
fn sd_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

// sd_probes() -- ID_SET_PROBE + k (docs/SETTINGS.md §6)
fn sd_probes(p: &mut Pn, ui: &mut Ui) {
    let st = &p.st;
    // 0 settings file, value 1 = persistence on
    sd_sem(ui, ID_SET_PROBE, &st.file, st.on as i32);
    // 1 files written; text "ok" / "Fehler" (last ini save)
    sd_sem(ui, ID_SET_PROBE + 1, if st.err == 0 { "ok" } else { "Fehler" }, st.st.saves as i32);
    // 2 a settings file was read
    sd_sem(ui, ID_SET_PROBE + 2, "", st.st.loaded as i32);
    // 3 theme setting; text = the theme shown
    let t = if ui.theme_dark() { "Dunkel" } else { "Hell" };
    sd_sem(ui, ID_SET_PROBE + 3, t, st.st.theme as i32);
    // 4 accent
    sd_sem(ui, ID_SET_PROBE + 4, &format!("{:06X}", st.st.accent), st.st.accent as i32);
    // 5 animations setting; text "anim_off=N"
    sd_sem(ui, ID_SET_PROBE + 5, &format!("anim_off={}", ui.anim_off() as i32), st.st.anim as i32);
    // 6 default tool (name)
    sd_sem(ui, ID_SET_PROBE + 6, sd_nth(&crate::preferences::TOOL_NAMES, st.st.deftool), st.st.deftool as i32);
    // 7 current tool
    sd_sem(ui, ID_SET_PROBE + 7, "", crate::tools::tool_cur(&p.tool));
    // 8 stored tool defaults, 9 the tool bar now
    let t = fmt_cap(crate::preferences::st_tc_text(&st.st.tooldef), PROBE_CAP);
    sd_sem(ui, ID_SET_PROBE + 8, &t, st.st.tooldef.hard as i32);
    let tc = crate::tools::tool_cfg(&p.tool);
    let t = fmt_cap(crate::preferences::st_tc_text(tc), PROBE_CAP);
    sd_sem(ui, ID_SET_PROBE + 9, &t, tc.hard as i32);
    // 10 primary secondary, value palette size
    let pal = &p.pal;
    let t = format!("{:08X} {:08X}", crate::workspace::palette::pal_primary(pal), crate::workspace::palette::pal_secondary(pal));
    sd_sem(ui, ID_SET_PROBE + 10, &t, crate::workspace::palette::pal_count(pal));
    // 11 first and last palette colour
    let n = crate::workspace::palette::pal_count(pal);
    let last = crate::workspace::palette::pal_entry(pal, if n != 0 { n - 1 } else { 0 });
    let t = format!("{:08X} {:08X}", crate::workspace::palette::pal_entry(pal, 0), last);
    sd_sem(ui, ID_SET_PROBE + 11, &t, 0);
    // 12 view: value units, text grid / rulers
    let v = &p.view;
    let t = format!("grid={} rulers={}", crate::workspace::view::view_grid(v) as i32, crate::workspace::view::view_rulers(v) as i32);
    sd_sem(ui, ID_SET_PROBE + 12, &t, crate::workspace::view::view_units(v) as i32);
    // 13 history budget setting (MB), text = spill folder setting
    sd_sem(ui, ID_SET_PROBE + 13, &st.st.tempdir, st.st.budget_mb as i32);
    // 14 threads setting, text "budget=N" (hist_budget in MB)
    let t = format!("budget={}", crate::document::history::memory::hist_budget(&p.hm) >> 20);
    sd_sem(ui, ID_SET_PROBE + 14, &t, st.st.threads as i32);
    // 15 recent files: value count, text the newest
    let t = String::from(crate::integrations::file_io::fio_recent_path(&p.fio, 0).unwrap_or(""));
    sd_sem(ui, ID_SET_PROBE + 15, &t, crate::integrations::file_io::fio_recent_count(&p.fio));
    // 16 main window "x y w h max" (normal rectangle)
    let m = st.mrect;
    let t = format!("{} {} {} {} {}", m.x, m.y, m.w, m.h, st.mmax as i32);
    sd_sem(ui, ID_SET_PROBE + 16, &t, 0);
    // 17 dialog page, text = its name
    let pg = p.dset.page;
    let t = if (pg as u32) < SP_COUNT as u32 { PAGE_NAMES[pg as usize] } else { "" };
    sd_sem(ui, ID_SET_PROBE + 17, t, pg);
    // 18 diagnostics text length
    sd_sem(ui, ID_SET_PROBE + 18, "", p.dset.diag.len() as i32);
}
