// boot -- what build_rs.py builds into a program, handed to the platform layer before the
// program's own code runs
//
//   Boot { app, manifest, assets }   rt::main! fills it at compile time: JFGUI_APP (the app's
//                                    folder name), JFGUI_PERMISSIONS (apps/NAME/PERMISSIONS or
//                                    PERMISSIONS.<bin>), JFGUI_ASSETS (--cfg jfgui_assets, JFA1)
//   set_handler(f)                   (plat's process entry) f sets the program up: its name,
//                                    app://, its permissions
//   start(&boot) -> Result<(), i32>  (rt::main!) the handler's verdict; Err = exit code.  Without
//                                    a handler a manifest cannot be enforced: exit code 2
//
// rt knows nothing of plat; plat registers itself here, so console programs get the same
// start-up as ui::main! without the UI crate.

use crate::sync::SpinLock;

/// what build_rs.py built into the program
pub struct Boot {
    /// the app's name for its folders (plat::vfs::set_app_id)
    pub app: Option<&'static str>,
    /// the app's permission manifest (plat::policy::install)
    pub manifest: Option<&'static str>,
    /// the app's assets, a JFA1 bundle (app://); empty without assets
    pub assets: &'static [u8],
}

/// sets the program up -> Err(exit code)
pub type Handler = fn(&Boot) -> Result<(), i32>;

static HANDLER: SpinLock<Option<Handler>> = SpinLock::new(None);

/// (plat's process entry) the function that sets a program up
pub fn set_handler(f: Handler) {
    *HANDLER.lock() = Some(f);
}

/// (rt::main!) set the program up before its own code runs
pub fn start(b: &Boot) -> Result<(), i32> {
    let h = *HANDLER.lock();
    match h {
        Some(f) => f(b),
        None if b.manifest.is_some() => {
            crate::os::write_stderr(b"error: no platform layer to install the permissions\n");
            Err(2)
        }
        None => Ok(()),
    }
}
