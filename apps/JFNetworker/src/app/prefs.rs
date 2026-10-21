// prefs -- the user's settings: a typed schema over config://settings.ini (base::settings), the
// values the window reads every frame, the language and the motion they choose
//
//   SCHEMA                     Ansicht: Raster zeigen, Rasterweite, Am Raster einrasten,
//                              Minimap; Simulation: Tempo beim Start, Bewegung; Allgemein:
//                              Sprache, automatisch speichern (Sekunden, 0 aus); the recent
//                              files live in the same file outside the schema
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

static SCHEMA: Schema = Schema::new(1, &[GRID.d, GRID_STEP.d, SNAP.d, MINIMAP.d, TEMPO.d, MOTION.d, LANG.d, AUTOSAVE.d])
    .sections(&[("view", "Ansicht"), ("sim", "Simulation"), ("app", "Allgemein"), ("files", "Dateien")]);

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
    }

    /// the toggles of the Ansicht menu into the settings
    pub(crate) fn push(&mut self) {
        let (g, sn, m) = (self.grid, self.snap, self.minimap);
        self.s.set(&GRID, g);
        self.s.set(&SNAP, sn);
        self.s.set(&MINIMAP, m);
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
