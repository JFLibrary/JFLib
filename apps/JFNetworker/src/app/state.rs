// state -- Net, the app's state: the project, the files, the view settings and the parts of
// the window
//
//   Net::new()              everything empty; run() puts it into an Rc<RefCell> and sets `me`
//   init(ui)                icons, the consoles' font, the canvas' painter, the user's
//                           settings (prefs.rs) with the language (tr.rs), --open FILE, the
//                           recovery of an autosaved project (doc/autosave.rs);
//                           --gen-topology N (a synthetic project without a file) and
//                           --traffic K (K pings at once) for tests
//   after_load()            a new or loaded project: canvas, inspector, consoles, log and
//                           device windows start over, the simulator runs in real time
//   save_prefs(ui)          settings, recent files and the dock layout (UIC_EXIT)
//   sim_timer(ui)           sets the simulator's timer when it needs one (every frame)
//
// The canvas paints at render time through a Weak to this record (canvas/paint.rs).  Headless
// runs (--script, --shot) neither read nor write the user's settings.  Settings live in
// config://settings.ini (with the recent files), the dock layout in config://dock.txt; the
// window's placement is AF_SAVE_STATE's (config://windows.ini).

use alloc::rc::{Rc, Weak};
use alloc::string::String;
use core::cell::RefCell;

use ui::*;

use crate::app::prefs::Prefs;
use crate::doc::autosave::Autosave;
use crate::doc::console::Consoles;
use crate::doc::file::{self, Files};
use crate::doc::Doc;
use crate::workspace::canvas::paint::Painter;
use crate::workspace::canvas::Canvas;
use crate::workspace::devwin::DevWins;
use crate::workspace::icons::{self, Symbols};
use crate::workspace::inspector::Inspector;
use crate::workspace::logview::Log;
use crate::workspace::metrics::Metrics;
use crate::workspace::palette::Palette;
use crate::workspace::shell::Shell;
use crate::workspace::sim::SimUi;

const DOCK: &str = "config://dock.txt";

/// the consoles' font: DejaVu Sans Mono (licence: lib/doc_view/assets/DejaVu-LICENSE.txt)
static MONO: &[u8] = include_bytes!("../../../../lib/doc_view/assets/DejaVuSansMono.ttf");

pub(crate) struct Net {
    /// the Rc of this record (render-time drawings keep a Weak and borrow it)
    pub me: Weak<RefCell<Net>>,
    pub doc: Doc,
    pub file: Files,
    pub prefs: Prefs,
    pub syms: Rc<Symbols>,
    pub canvas: Canvas,
    pub palette: Palette,
    pub inspector: Inspector,
    pub shell: Shell,
    /// the consoles of the devices and the Terminal panel's tabs
    pub cons: Consoles,
    /// the devices' log lines (Protokoll panel)
    pub log: Log,
    pub devwin: DevWins,
    /// the simulation mode: mode, steps, selection, the Ereignisse and Paket panels, PDUs
    pub sim: SimUi,
    /// autosave and the recovery of an autosaved project (doc/autosave.rs)
    pub autosave: Autosave,
    /// the settings dialog and the overview of the shortcuts are open
    pub settings_open: Shared<i32>,
    pub keys_open: Shared<i32>,
    /// the lab of the project: its goals over the run, the hints shown, the list of labs
    /// (workspace/task.rs)
    pub task: crate::workspace::task::Task,
    /// the Metriken panel of the lab (workspace/metrics.rs)
    pub metrics: Metrics,
}

impl Net {
    pub(crate) fn new() -> Net {
        let syms = Rc::new(Symbols::new());
        let palette = Palette::new(&syms);
        Net {
            me: Weak::new(),
            doc: Doc::new(),
            file: Files::new(),
            prefs: Prefs::new(),
            syms,
            canvas: Canvas::new(),
            palette,
            inspector: Inspector::new(),
            shell: Shell::new(),
            cons: Consoles::new(),
            log: Log::new(),
            devwin: DevWins::new(),
            sim: SimUi::new(),
            autosave: Autosave::new(),
            settings_open: Shared::new(0),
            keys_open: Shared::new(0),
            task: crate::workspace::task::Task::new(),
            metrics: Metrics::new(),
        }
    }

    /// App::init: the icons, the painter, the user's settings, a project from the command
    /// line (--open FILE, or a .jfnet path)
    pub(crate) fn init(&mut self, ui: &mut Ui) -> bool {
        icons::install(ui);
        let mono = ui.text.load_mem(MONO, 0);
        if mono >= 0 {
            ui.mono_font_set(mono as u32);
        }
        let painter: Rc<dyn DgPaint> = Rc::new(Painter { net: self.me.clone() });
        self.canvas.painter = Some(painter);
        // the settings (headless: the defaults in memory), the language, the motion
        crate::app::tr::install();
        self.prefs = Prefs::open(ui.headless());
        self.prefs.apply(ui);
        self.sim.tempo = self.prefs.tempo;
        self.doc.proj.layout.grid = self.prefs.grid_step;
        if !ui.headless() {
            self.load_prefs();
        }
        self.after_load();
        let args = plat::args();
        let open = plat::arg_value("--open").or_else(|| args.iter().skip(1).find(|a| !a.starts_with('-') && a.to_lowercase().ends_with(".jfnet")).cloned());
        if let Some(path) = open {
            file::open_path(self, ui, &path);
        }
        // an autosaved project of a session that did not end well: the recovery bar
        crate::doc::autosave::startup(self, ui);
        // performance tests: a synthetic project (--gen-topology N), K pings at once (--traffic K)
        let num = |a: &str| plat::arg_value(a).and_then(|v| v.trim().parse::<usize>().ok());
        if let Some(k) = num("--gen-topology") {
            file::open_generated(self, ui, k);
        }
        if let Some(k) = num("--traffic") {
            self.doc.sim.with_engine(|e| e.ping_many(k));
        }
        true
    }

    /// a new or loaded project: no selection, its view, fresh edit buffers, the simulator
    /// running in real time
    pub(crate) fn after_load(&mut self) {
        self.canvas.reset(&self.doc.proj);
        self.inspector.reset();
        self.doc.sim.start();
        // the consoles, the log and the device windows belong to the old project
        self.cons = Consoles::new();
        self.log = Log::new();
        self.devwin = DevWins::new();
        self.metrics = Metrics::new();
        crate::workspace::sim::loaded(self, false);
        // the lab's goals start over (its panel shows)
        crate::workspace::task::loaded(self);
        // the copy of the project that went is no longer needed
        crate::doc::autosave::clean(self);
    }

    // the recent files and the dock layout of the user
    fn load_prefs(&mut self) {
        self.file.recent = self.prefs.recent();
        if let Some(d) = plat::read_file(DOCK).and_then(|b| DockState::load(&String::from_utf8_lossy(&b))) {
            self.shell.dock = d;
        }
    }

    /// the simulator's timer (TIMER_SIM -> App::command(UIC_TIMER)), when it needs one
    pub(crate) fn sim_timer(&mut self, ui: &mut Ui) {
        if let Some(ms) = self.doc.sim.arm() {
            ui.timer(crate::app::constants::TIMER_SIM, ms);
        }
    }

    /// UIC_EXIT: the settings, the recent files and the dock layout
    pub(crate) fn save_prefs(&mut self, ui: &Ui) {
        if ui.headless() || !self.prefs.kept {
            return;
        }
        let recent = self.file.recent.clone();
        self.prefs.save(&recent);
        plat::write_file(DOCK, self.shell.dock.save().as_bytes());
    }
}
