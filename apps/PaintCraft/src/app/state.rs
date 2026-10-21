//! Editor state and deferred UI updates.
use alloc::rc::Weak;
use alloc::vec::Vec;
use core::cell::RefCell;
use ui::{Command, Ui};

/// Shared editor state owned by the application runtime.
pub(crate) struct Pn {
    /// the Rc of this record (render-time callbacks keep a Weak and borrow Pn)
    pub(crate) me: Weak<RefCell<Pn>>,
    // Application lifecycle and deferred UI changes.
    /// the command table (pn_cmds in .data): labels / icons / keys changed at run time
    pub(crate) cmds: Vec<Command>,
    /// cmds changed since it was published to the UI
    pub(crate) cmds_dirty: bool,
    /// ui_repass / ui_invalidate requested by code without a Ui (flushed after the callback)
    pub(crate) want_repass: bool,
    pub(crate) want_invalidate: bool,
    // Document, workspace and preferences.
    pub(crate) doc: crate::document::model::Docs,
    pub(crate) lay: crate::document::layers::Lay,
    pub(crate) comp: crate::document::composite::Compose,
    pub(crate) view: crate::workspace::view::View,
    pub(crate) tabs: crate::workspace::tabs::Tabs,
    pub(crate) chrome: crate::workspace::chrome::Chrome,
    pub(crate) cur: crate::workspace::cursors::Cursors,
    pub(crate) icons: crate::workspace::icons::PnIcons,
    pub(crate) st: crate::preferences::St,
    // History and editing operations.
    pub(crate) hist: crate::document::history::Hist,
    pub(crate) hm: crate::document::history::memory::HistMem,
    pub(crate) hdiag: crate::document::history::diagnostics::HistDiag,
    pub(crate) sel: crate::document::selection::Sel,
    pub(crate) clip: crate::document::clipboard::Clip,
    pub(crate) iop: crate::document::operations::Iop,
    // Interactive tools and shared engines.
    pub(crate) tool: crate::tools::Tool,
    pub(crate) be: crate::tools::brushes::Be,
    pub(crate) tbrush: crate::tools::brush::TBrush,
    pub(crate) tpicker: crate::tools::picker::TPicker,
    pub(crate) tview: crate::tools::view::TView,
    pub(crate) twand: crate::tools::wand::TWand,
    pub(crate) tsel: crate::tools::select::TSel,
    pub(crate) tbucket: crate::tools::bucket::TBucket,
    pub(crate) tclone: crate::tools::clone::TClone,
    pub(crate) tgrad: crate::tools::gradient::TGrad,
    pub(crate) tmove: crate::tools::move_pixels::TMove,
    pub(crate) ttext: crate::tools::text::TText,
    pub(crate) tshp: crate::tools::shapes::TShapes,
    pub(crate) shl: crate::tools::shapes_library::Shl,
    // Panels and dialogs.
    pub(crate) dock: crate::workspace::panels::dock::Dock,
    pub(crate) wlay: crate::workspace::panels::layers::WLay,
    pub(crate) whist: crate::workspace::panels::history::WHist,
    pub(crate) wcol: crate::workspace::panels::colors::WCol,
    pub(crate) wtools: crate::workspace::panels::tools::WTools,
    pub(crate) pal: crate::workspace::palette::Pal,
    pub(crate) dlgnew: crate::workspace::dialogs::new_image::DlgNew,
    pub(crate) dimg: crate::workspace::dialogs::image::DImg,
    pub(crate) rz: crate::workspace::dialogs::rotate_zoom::Rz,
    pub(crate) lprops: crate::workspace::dialogs::layer_properties::LProps,
    pub(crate) misc: crate::workspace::dialogs::misc::Misc,
    pub(crate) dset: crate::workspace::dialogs::settings::DSet,
    pub(crate) saveopt: crate::workspace::dialogs::save_options::SaveOpt,
    pub(crate) fio: crate::integrations::file_io::Fio,
    // Effects and external integrations.
    pub(crate) fx: crate::effects::Fx,
    pub(crate) fxui: crate::effects::controls::FxUi,
    pub(crate) curves: crate::effects::curves::Curves,
    pub(crate) levels: crate::effects::levels::Levels,
    pub(crate) plug: crate::integrations::plugins::Plug,
    pub(crate) prt: crate::integrations::printing::Prt,
    pub(crate) cm: crate::document::color::Cm,
    pub(crate) pdn: crate::integrations::paint_dot_net::host::Host,
    pub(crate) gmic: crate::integrations::gmic::Host,
    pub(crate) custom_brush: crate::tools::brushes::custom::Custom,
    pub(crate) services: crate::integrations::services::Services,
    pub(crate) cmdlg: crate::document::color::CmDlg, // pn-sys: the dialog's Shared flags (Cm must be Sync)
    pub(crate) crash: crate::diagnostics::crash::Crash,
    pub(crate) diag: crate::diagnostics::Diag,
}

impl Pn {
    pub(super) fn new() -> Pn {
        Pn {
            me: Weak::new(),
            cmds: Vec::from(&crate::app::commands::PN_CMDS[..]),
            cmds_dirty: false,
            want_repass: false,
            want_invalidate: false,
            doc: Default::default(),
            lay: Default::default(),
            comp: Default::default(),
            view: Default::default(),
            tabs: Default::default(),
            chrome: Default::default(),
            cur: Default::default(),
            icons: Default::default(),
            st: Default::default(),
            hist: Default::default(),
            hm: Default::default(),
            hdiag: Default::default(),
            sel: Default::default(),
            clip: Default::default(),
            iop: Default::default(),
            tool: Default::default(),
            be: Default::default(),
            tbrush: Default::default(),
            tpicker: Default::default(),
            tview: Default::default(),
            twand: Default::default(),
            tsel: Default::default(),
            tbucket: Default::default(),
            tclone: Default::default(),
            tgrad: Default::default(),
            tmove: Default::default(),
            ttext: Default::default(),
            tshp: Default::default(),
            shl: Default::default(),
            dock: Default::default(),
            wlay: Default::default(),
            whist: Default::default(),
            wcol: Default::default(),
            wtools: Default::default(),
            pal: Default::default(),
            dlgnew: Default::default(),
            dimg: Default::default(),
            rz: Default::default(),
            lprops: Default::default(),
            misc: Default::default(),
            dset: Default::default(),
            saveopt: Default::default(),
            fio: Default::default(),
            fx: Default::default(),
            fxui: Default::default(),
            curves: Default::default(),
            levels: Default::default(),
            plug: Default::default(),
            prt: Default::default(),
            cm: Default::default(),
            pdn: Default::default(),
            gmic: Default::default(),
            custom_brush: Default::default(),
            services: Default::default(),
            cmdlg: Default::default(), // pn-sys
            crash: Default::default(),
            diag: Default::default(),
        }
    }

    /// ui_repass from code without a Ui: another layout pass (flushed after the callback)
    pub(crate) fn repass(&mut self) {
        self.want_repass = true;
    }

    /// ui_invalidate from code without a Ui: render again (flushed after the callback)
    pub(crate) fn invalidate(&mut self) {
        self.want_invalidate = true;
    }

    /// the end of every app callback: requested repass / invalidate, a changed command table
    pub(crate) fn flush(&mut self, ui: &mut Ui) {
        self.want_repass |= core::mem::take(&mut self.pal.repass); // pn-ui: palette setters (pal_changed)
        if self.want_repass {
            self.want_repass = false;
            ui.repass();
        }
        if self.want_invalidate {
            self.want_invalidate = false;
            ui.invalidate();
        }
        if self.cmds_dirty {
            self.cmds_dirty = false;
            // the library keeps a 'static table: publish a copy (labels change rarely:
            // recent files, "<Effekt> wiederholen", plugin rows)
            ui.desc.cmds = Vec::leak(self.cmds.clone());
        }
    }
}
