//! PaintCraft application entry and adapter for JFLib's UI runtime.
pub(crate) mod commands;
pub(crate) mod constants;
pub(crate) mod formatting;
mod lifecycle;
mod state;

pub(crate) use state::Pn;

use alloc::rc::Rc;
use core::cell::RefCell;
use lifecycle::{pn_cmd, pn_frame, pn_init};
use ui::*;

const DESC: AppDesc = AppDesc {
    title: "Unbenannt - PaintCraft",
    flags: AF_RESIZABLE | AF_CUSTOM_TITLEBAR | AF_SINGLE_INSTANCE, // later starts open their files here
    w: 1280,
    h: 800,
    min_w: 1000,
    min_h: 600,
    accent: 0,
    cmds: &crate::app::commands::PN_CMDS,
    menubar: &crate::app::commands::PN_MENUBAR,
};

/// the ui::App of PaintCraft: Pn behind an Rc<RefCell> (render-time callbacks borrow it)
struct PnApp {
    p: Rc<RefCell<Pn>>,
}

impl App for PnApp {
    // pn_init() -> 0
    fn init(&mut self, ui: &mut Ui) -> bool {
        let mut p = self.p.borrow_mut();
        pn_init(&mut p, ui);
        p.flush(ui);
        true
    }

    // pn_frame()
    fn frame(&mut self, ui: &mut Ui) {
        let mut p = self.p.borrow_mut();
        pn_frame(&mut p, ui);
        p.flush(ui);
    }

    // pn_cmd(id, arg) -> handled
    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        let mut p = self.p.borrow_mut();
        let r = pn_cmd(&mut p, ui, id, arg);
        p.flush(ui);
        r
    }
}

/// the app record and the entry point
pub(crate) fn run() -> i32 {
    let p = Rc::new(RefCell::new(Pn::new()));
    p.borrow_mut().me = Rc::downgrade(&p);
    let app = PnApp { p };
    ui::launch(&DESC, app, &style::STYLE)
}
