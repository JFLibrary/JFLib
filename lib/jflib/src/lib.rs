//! JFLib: a GUI library and everything it stands on, without the standard library.
//! One dependency brings all of it; a program links only what it uses.
//!
//! `jflib::style` is the style of the environment variable `JFLIB_STYLE`, without it `winui`.
#![no_std]
#![forbid(unsafe_code)]

pub use rt;
pub use agent_core;
pub use plat;
pub use base;
pub use agent_io;
pub use simd;
pub use gfx;
pub use img_core;
pub use archive;
pub use doc_core;
pub use doc_layout;
pub use doc_io;
pub use img_png;
pub use text_shape;
pub use text;
pub use img_jpeg;
pub use img_bmp;
pub use img_gif;
pub use img_tga;
pub use img_tiff;
pub use img_webp;
pub use img_hevc;
pub use img_av1;
pub use img_heif;
pub use img_avif;
pub use img_jxl;
pub use img_jxr;
pub use img_dds;
pub use img_pdn;
pub use fx;
pub use img_svg;
pub use img_all;
pub use spell;
pub use ui;
pub use doc_view;
pub use datetime;
pub use crypto;
pub use compress;
pub use shader;
pub use wgsl;
pub use netpkt;
pub use gpu;
pub use eapm;
pub use style_assets;
pub use style_winui;
pub use style_classic;
pub use style_kde;
pub use style_macos;

// plat has no fs and net on these two targets
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub use cas;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub use datastore;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub use netio;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub use update;

// takes about 25 minutes for wasm32 (docs/CRATES.md, "Risiken")
#[cfg(not(target_arch = "wasm32"))]
pub use img_jxl_enc;

#[cfg(jflib_style = "winui")]
pub use style_winui as style;
#[cfg(jflib_style = "classic")]
pub use style_classic as style;
#[cfg(jflib_style = "kde")]
pub use style_kde as style;
#[cfg(jflib_style = "macos")]
pub use style_macos as style;
