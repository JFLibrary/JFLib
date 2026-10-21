//! Original paint.net plugin catalog and host bridge.
pub(crate) mod catalog;
pub(crate) mod host;
#[cfg(any(windows, target_os = "linux"))]
mod paths;
#[cfg(any(windows, target_os = "linux"))]
pub(crate) mod transport;
