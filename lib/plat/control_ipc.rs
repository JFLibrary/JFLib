// Explicit, user-private local control channels. New native platform adapter.
// The endpoint is created only by Listener::bind; importing this module opens nothing.

#[cfg(windows)]
#[path = "control_ipc_win.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "control_ipc_linux.rs"]
mod native;
#[cfg(target_vendor = "apple")]
#[path = "control_ipc_apple.rs"]
mod native;

#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub use native::{Listener, Stream};
// the app's single instance (instance.rs)
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub(crate) use native::{InstConn, inst_connect, inst_listen};
