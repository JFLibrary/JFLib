//! Platform-selected native plugin implementation and the fallback of the browser and Android
//! (the plugin ABI is x86-64 "win64": native.rs).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod native;
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
mod web;

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub(crate) use native::*;
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub(crate) use web::*;
