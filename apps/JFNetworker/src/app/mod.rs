// app -- JFNetworker's App: the window description, the adapter to ui::App, the start
//
//   run()      --selftest checks the data model and the simulator and ends (tests/model.uis);
//              PROJECT --scenario S runs a scenario without a window and ends with its
//              report (tests/scen_*.uis); otherwise the main window (ui::launch)
//
// The state (Net, state.rs) sits behind an Rc<RefCell>: the canvas paints at render time and
// borrows it through a Weak.  commands.rs has the command table, the menus and what the
// commands do; constants.rs the ids.

pub(crate) mod commands;
pub(crate) mod constants;
pub(crate) mod state;

use alloc::rc::Rc;
use core::cell::RefCell;

use ui::*;

pub(crate) use state::Net;

const DESC: AppDesc = AppDesc {
    title: "JFNetworker",
    flags: AF_RESIZABLE | AF_SAVE_STATE,
    w: 1280,
    h: 800,
    min_w: 800,
    min_h: 520,
    cmds: &commands::CMDS,
    menubar: &commands::MENUBAR,
    ..AppDesc::NONE
};

/// the ui::App: Net behind an Rc<RefCell> (render-time drawings borrow it)
struct NetApp {
    n: Rc<RefCell<Net>>,
}

impl App for NetApp {
    fn init(&mut self, ui: &mut Ui) -> bool {
        self.n.borrow_mut().init(ui)
    }

    fn frame(&mut self, ui: &mut Ui) {
        let mut n = self.n.borrow_mut();
        crate::workspace::frame(&mut n, ui);
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        let mut n = self.n.borrow_mut();
        commands::command(&mut n, ui, id, arg)
    }
}

/// the entry point (ui::main!)
pub(crate) fn run() -> i32 {
    if plat::args().iter().any(|a| a == "--selftest") {
        return crate::selftest::run();
    }
    // PROJECT --scenario S: a scenario without a window (crates/jfn_run, tests/scen)
    if let Some(code) = jfn_run::gui_args(&plat::args()) {
        return code;
    }
    let n = Rc::new(RefCell::new(Net::new()));
    n.borrow_mut().me = Rc::downgrade(&n);
    ui::launch(&DESC, NetApp { n }, &style::STYLE)
}
