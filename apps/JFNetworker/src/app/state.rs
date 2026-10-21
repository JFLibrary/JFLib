// state -- Net, the app's state: the project, the files, the view settings and the parts of
// the window
//
//   Net::new()              everything empty; run() puts it into an Rc<RefCell> and sets `me`
//   init(ui)                icons, the consoles' font, the canvas' painter, the user's
//                           settings, --open FILE
//   after_load()            a new or loaded project: canvas, inspector, consoles, log and
//                           device windows start over, the simulator runs in real time
//   save_prefs(ui)          settings, recent files and the dock layout (UIC_EXIT)
//   sim_timer(ui)           sets the simulator's timer when it needs one (every frame)
//
// The canvas paints at render time through a Weak to this record (canvas/paint.rs).  Headless
// runs (--script, --shot) neither read nor write the user's settings.  Settings live in
// config://settings.ini, the dock layout in config://dock.txt; the window's placement is
// AF_SAVE_STATE's (config://windows.ini).

use alloc::rc::{Rc, Weak};
use alloc::string::String;
use core::cell::RefCell;

use ui::*;

use crate::doc::console::Consoles;
use crate::doc::file::{self, Files};
use crate::doc::Doc;
use crate::workspace::canvas::paint::Painter;
use crate::workspace::canvas::Canvas;
use crate::workspace::devwin::DevWins;
use crate::workspace::icons::{self, Symbols};
use crate::workspace::inspector::Inspector;
use crate::workspace::logview::Log;
use crate::workspace::palette::Palette;
use crate::workspace::shell::Shell;
use crate::workspace::sim::SimUi;

const SETTINGS: &str = "config://settings.ini";
const DOCK: &str = "config://dock.txt";

/// the consoles' font: DejaVu Sans Mono (licence: lib/doc_view/assets/DejaVu-LICENSE.txt)
static MONO: &[u8] = include_bytes!("../../../../lib/doc_view/assets/DejaVuSansMono.ttf");

/// view settings of the app (not of a project)
pub(crate) struct Prefs {
    /// the grid is shown
    pub grid: bool,
    /// moves and placing snap to the grid
    pub snap: bool,
    pub minimap: bool,
    /// they were read from the user's settings (and are written back at the end)
    pub kept: bool,
}

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
}

impl Net {
    pub(crate) fn new() -> Net {
        let syms = Rc::new(Symbols::new());
        let palette = Palette::new(&syms);
        Net {
            me: Weak::new(),
            doc: Doc::new(),
            file: Files::new(),
            prefs: Prefs { grid: true, snap: true, minimap: false, kept: false },
            syms,
            canvas: Canvas::new(),
            palette,
            inspector: Inspector::new(),
            shell: Shell::new(),
            cons: Consoles::new(),
            log: Log::new(),
            devwin: DevWins::new(),
            sim: SimUi::new(),
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
        if !ui.headless() {
            self.load_prefs();
        }
        self.after_load();
        let args = plat::args();
        let open = plat::arg_value("--open").or_else(|| args.iter().skip(1).find(|a| !a.starts_with('-') && a.to_lowercase().ends_with(".jfnet")).cloned());
        if let Some(path) = open {
            file::open_path(self, ui, &path);
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
        crate::workspace::sim::loaded(self, false);
    }

    fn load_prefs(&mut self) {
        let ini = base::Ini::load(SETTINGS);
        let on = |k: &str, def: bool| ini.get("view", k).map_or(def, |v| v != "0");
        self.prefs = Prefs { grid: on("grid", true), snap: on("snap", true), minimap: on("minimap", false), kept: true };
        self.file.recent = ini.get("files", "recent").unwrap_or("").split('|').filter(|s| !s.is_empty()).map(String::from).collect();
        self.file.recent.truncate(crate::app::constants::RECENT_MAX);
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
    pub(crate) fn save_prefs(&self, ui: &Ui) {
        if ui.headless() || !self.prefs.kept {
            return;
        }
        let mut ini = base::Ini::load(SETTINGS);
        let b = |v: bool| if v { "1" } else { "0" };
        ini.set("view", "grid", b(self.prefs.grid));
        ini.set("view", "snap", b(self.prefs.snap));
        ini.set("view", "minimap", b(self.prefs.minimap));
        ini.set("files", "recent", &self.file.recent.join("|"));
        ini.save(SETTINGS);
        plat::write_file(DOCK, self.shell.dock.save().as_bytes());
    }
}
