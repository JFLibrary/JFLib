// prefs -- the user's settings: a typed schema over config://settings.ini (base::settings), the
// values the window reads every frame, the language and the motion they choose
//
//   SCHEMA                     Ansicht: Raster zeigen, Rasterweite, Am Raster einrasten,
//                              Minimap; Simulation: Tempo beim Start, Bewegung; Allgemein:
//                              Sprache, automatisch speichern (Sekunden, 0 aus); Desktop (the
//                              hosts' desktops, V2): Hintergrund, Größe der Symbole, Uhr der
//                              Taskleiste; Startseite: Beim Start anzeigen; Akademie: Adresse der
//                              Kurse; the recent files live in the same file outside the schema
//   DESK_*_COUNT, desk_names(k)   the desktop's choices for its Einstellungen program
//   Prefs::open(headless)      the user's file; headless runs (--script, --shot) keep them in
//                              memory only (Settings::new), so tests never read or write them
//   prefs.pull() / push()      the typed fields from the settings / into them (the toggles of
//                              the Ansicht menu)
//   prefs.apply(ui)            what the window takes over at once: language (tr::apply),
//                              reduced motion
//   prefs.save(recent)         the file with the recent files (after the dialog, UIC_EXIT)
//
// The settings dialog (workspace/settings.rs) shows the page ui.settings_page builds from the
// schema; its labels and help texts go through the app's catalogue (English).

use alloc::string::String;
use alloc::vec::Vec;

use base::{Key, Schema, Settings};
use ui::*;

pub(crate) const SETTINGS: &str = "config://settings.ini";

const GRID: Key<bool> = Key::bool("view.grid", true).label("Raster zeigen");
const GRID_STEP: Key<i64> = Key::int("view.grid_step", 20, 4, 200)
    .label("Rasterweite")
    .step(4)
    .help("In Weltpunkten (bei 100 % je ein DIP); für das offene und neue Projekte.");
const SNAP: Key<bool> = Key::bool("view.snap", true).label("Am Raster einrasten");
const MINIMAP: Key<bool> = Key::bool("view.minimap", false).label("Minimap zeigen");
const TEMPOS: [(&str, &str); 6] = [("0.25", "0,25×"), ("0.5", "0,5×"), ("1", "1×"), ("2", "2×"), ("4", "4×"), ("8", "8×")];
const TEMPO: Key<usize> = Key::choice("sim.tempo", 2, &TEMPOS)
    .label("Tempo beim Start")
    .help("Wie schnell die Zeit läuft; im Simulationsmodus der Takt der Schritte.");
const MOTIONS: [(&str, &str); 3] = [("system", "Wie das System"), ("reduced", "Reduziert"), ("full", "Volle Bewegung")];
const MOTION: Key<usize> = Key::choice("view.motion", 0, &MOTIONS)
    .label("Bewegung")
    .help("Reduziert: Pakete springen von Schritt zu Schritt, nichts blendet.");
const LANGS: [(&str, &str); 3] = [("system", "Wie das System"), ("de", "Deutsch"), ("en", "English")];
const LANG: Key<usize> = Key::choice("app.lang", 0, &LANGS).label("Sprache");
const AUTOSAVE: Key<i64> = Key::int("files.autosave", 120, 0, 3600)
    .label("Automatisch sichern alle (Sekunden)")
    .step(30)
    .help("Eine Kopie ungespeicherter Änderungen für einen Absturz; 0 schaltet es aus.");

// the desktop of hosts (V2, workspace/devwin/desktop.rs): the look of the user's desktops, no
// setting of a simulated device
const DESK_BGS: [(&str, &str); 6] = [("slate", "Schiefer"), ("sage", "Salbei"), ("sand", "Sand"), ("night", "Nachtblau"), ("dots", "Punktraster"), ("net", "Netz")];
const DESK_BG: Key<usize> = Key::choice("desk.background", 0, &DESK_BGS)
    .label("Hintergrund")
    .help("Der Hintergrund der Desktops aller Hosts; im Hochkontrast eine ruhige Fläche.");
const DESK_ICONS: [(&str, &str); 3] = [("small", "Klein"), ("medium", "Mittelgroß"), ("large", "Groß")];
const DESK_ICON: Key<usize> = Key::choice("desk.icons", 1, &DESK_ICONS).label("Größe der Symbole");
const DESK_CLOCKS: [(&str, &str); 3] = [("hm", "Stunden und Minuten"), ("hms", "Mit Sekunden"), ("date", "Mit Datum")];
const DESK_CLOCK: Key<usize> = Key::choice("desk.clock", 0, &DESK_CLOCKS)
    .label("Uhr der Taskleiste")
    .help("Sie zeigt die Uhr des Geräts in der Simulation.");

// the start page (ST1, workspace/startpage.rs)
const START_PAGE: Key<bool> = Key::bool("start.show", true)
    .label("Beim Start anzeigen")
    .help("Ohne diese Seite beginnt das Programm mit einem leeren Projekt.");

// the Akademie (AK3, workspace/academy/install.rs)
const ACADEMY_BASE: Key<String> = Key::text("academy.base", jfn_run::install::DEFAULT_BASE, 500)
    .label("Adresse der Kurse") // i18n
    .help("Von hier lädt „Nach Kursen suchen“ die Kursliste und die Kurse. Erlaubt ist nur der Host raw.githubusercontent.com; der Knopf daneben führt zur Vorgabe zurück."); // i18n

static SCHEMA: Schema = Schema::new(1, &[GRID.d, GRID_STEP.d, SNAP.d, MINIMAP.d, TEMPO.d, MOTION.d, LANG.d, AUTOSAVE.d, DESK_BG.d, DESK_ICON.d, DESK_CLOCK.d, START_PAGE.d, ACADEMY_BASE.d])
    .sections(&[("view", "Ansicht"), ("sim", "Simulation"), ("app", "Allgemein"), ("files", "Dateien"), ("desk", "Desktop"), ("start", "Startseite"), ("academy", "Akademie")]);

/// the backgrounds, icon sizes and clocks of the desktop (DESK_BGS, DESK_ICONS, DESK_CLOCKS)
pub(crate) const DESK_BG_COUNT: usize = DESK_BGS.len();
pub(crate) const DESK_ICON_COUNT: usize = DESK_ICONS.len();
pub(crate) const DESK_CLOCK_COUNT: usize = DESK_CLOCKS.len();

/// the names of the desktop's choices (for its Einstellungen program; i18n through t())
pub(crate) fn desk_names(which: usize) -> Vec<&'static str> {
    let list: &[(&str, &str)] = match which {
        0 => &DESK_BGS,
        1 => &DESK_ICONS,
        _ => &DESK_CLOCKS,
    };
    list.iter().map(|x| x.1).collect()
}

/// the motion setting: as the system, reduced, full
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    System,
    Reduced,
    Full,
}

/// the language setting: as the system, German, English
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lang {
    System,
    De,
    En,
}

/// the user's settings and their typed values
pub(crate) struct Prefs {
    pub s: Settings,
    /// the grid is shown
    pub grid: bool,
    /// the grid step of projects without their own (world units)
    pub grid_step: u32,
    /// moves and placing snap to the grid
    pub snap: bool,
    pub minimap: bool,
    /// the speed at the start: an index into workspace::sim::SPEEDS
    pub tempo: usize,
    pub motion: Motion,
    pub lang: Lang,
    /// seconds between autosaves (0: off)
    pub autosave_s: u32,
    /// the desktop of hosts: the background, the icons' size, the clock (indices into
    /// DESK_BGS, DESK_ICONS, DESK_CLOCKS)
    pub desk_bg: usize,
    pub desk_icons: usize,
    pub desk_clock: usize,
    /// the window opens on the start page (when nothing was opened)
    pub start_page: bool,
    /// where the Akademie looks for its courses: the folder of index.json
    pub academy_base: String,
    /// they belong to the user's file (not a headless run)
    pub kept: bool,
}

impl Prefs {
    /// the defaults in memory (before init; headless runs keep them)
    pub(crate) fn new() -> Prefs {
        let mut p = Prefs {
            s: Settings::new(&SCHEMA),
            grid: true,
            grid_step: 20,
            snap: true,
            minimap: false,
            tempo: 2,
            motion: Motion::System,
            lang: Lang::System,
            autosave_s: 120,
            desk_bg: 0,
            desk_icons: 1,
            desk_clock: 0,
            start_page: true,
            academy_base: String::new(),
            kept: false,
        };
        p.pull();
        p
    }

    /// the user's settings (headless: the defaults in memory)
    pub(crate) fn open(headless: bool) -> Prefs {
        let mut p = Prefs::new();
        if !headless {
            p.s = Settings::open(SETTINGS, &SCHEMA);
            p.kept = true;
            p.pull();
        }
        p
    }

    /// the typed fields from the settings
    pub(crate) fn pull(&mut self) {
        let s = &self.s;
        self.grid = s.get(&GRID);
        self.grid_step = s.get(&GRID_STEP) as u32;
        self.snap = s.get(&SNAP);
        self.minimap = s.get(&MINIMAP);
        self.tempo = s.get(&TEMPO);
        self.motion = match s.get(&MOTION) {
            1 => Motion::Reduced,
            2 => Motion::Full,
            _ => Motion::System,
        };
        self.lang = match s.get(&LANG) {
            1 => Lang::De,
            2 => Lang::En,
            _ => Lang::System,
        };
        self.autosave_s = s.get(&AUTOSAVE) as u32;
        self.desk_bg = s.get(&DESK_BG);
        self.desk_icons = s.get(&DESK_ICON);
        self.desk_clock = s.get(&DESK_CLOCK);
        self.start_page = s.get(&START_PAGE);
        self.academy_base = s.get(&ACADEMY_BASE);
    }

    /// another address for the courses (selftest; the window's way is the settings page)
    pub(crate) fn set_academy_base(&mut self, url: &str) {
        self.s.set(&ACADEMY_BASE, String::from(url));
        self.pull();
    }

    /// the address of the courses back at its default (the page's reset button does the same)
    pub(crate) fn reset_academy_base(&mut self) {
        self.s.reset(&ACADEMY_BASE);
        self.pull();
    }

    /// the toggles of the Ansicht menu and the desktop's Einstellungen program into the
    /// settings
    pub(crate) fn push(&mut self) {
        let (g, sn, m) = (self.grid, self.snap, self.minimap);
        self.s.set(&GRID, g);
        self.s.set(&SNAP, sn);
        self.s.set(&MINIMAP, m);
        let (b, i, c) = (self.desk_bg, self.desk_icons, self.desk_clock);
        self.s.set(&DESK_BG, b);
        self.s.set(&DESK_ICON, i);
        self.s.set(&DESK_CLOCK, c);
    }

    /// the language and the motion for the window (at the start and after the dialog)
    pub(crate) fn apply(&self, ui: &mut Ui) {
        crate::app::tr::apply(ui, self.lang);
        ui.set_reduced_motion(match self.motion {
            Motion::System => None,
            Motion::Reduced => Some(true),
            Motion::Full => Some(false),
        });
    }

    /// the most recent files (from the same file)
    pub(crate) fn recent(&self) -> Vec<String> {
        let list = self.s.ini().get("files", "recent").unwrap_or("");
        list.split('|').filter(|x| !x.is_empty()).map(String::from).take(crate::app::constants::RECENT_MAX).collect()
    }

    /// write the file (with the recent files); nothing for a headless run
    pub(crate) fn save(&mut self, recent: &[String]) {
        if !self.kept {
            return;
        }
        self.push();
        let list = recent.join("|");
        if self.s.ini().get("files", "recent").unwrap_or("") != list {
            self.s.ini_mut().set("files", "recent", &list);
        }
        if self.s.dirty() {
            self.s.save();
        }
    }
}
