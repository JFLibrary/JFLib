// The crate's page (docs.rs) is its README, the same text crates.io shows.  `jflib::style` is the
// style of the environment variable `JFLIB_STYLE`, without it `winui`.
//
// What this crate promises (docs/VERSIONING.md, "The crates on crates.io") is every crate below
// by its path here, `jflib::ui::...`, as docs/API.md documents it; of the styles their `STYLE`.
// `simd`, `text_shape` and `style_assets` are building blocks the others stand on: reachable,
// hidden, no part of that promise.
#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]

/// How an app is written with JFLib: the guide of the repository (`docs/GUIDE.md`), which also
/// lies in this crate's package as `docs/GUIDE.md`.  Only documentation, no code: it exists when
/// rustdoc builds the docs (docs.rs), its links lead to the repository on GitHub (build.rs).
#[cfg(doc)]
#[doc = include_str!(concat!(env!("OUT_DIR"), "/GUIDE.md"))]
pub mod guide {}

pub use rt;
pub use agent_core;
pub use plat;
pub use base;
pub use agent_io;
#[doc(hidden)]
pub use simd;
pub use gfx;
pub use img_core;
pub use archive;
pub use doc_core;
pub use doc_layout;
pub use doc_io;
pub use img_png;
#[doc(hidden)]
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
pub use render3d;
#[doc(hidden)]
pub use style_assets;

/// WinUI 3 (Fluent): `STYLE` for `ui::launch` and the style switch at run time
pub mod style_winui {
    pub use ::style_winui::STYLE;
}

/// the classic look: grey 3D edges, dark blue title bars
pub mod style_classic {
    pub use ::style_classic::STYLE;
}

/// KDE Plasma (Breeze)
pub mod style_kde {
    pub use ::style_kde::STYLE;
}

/// macOS: white buttons, blue selection, the traffic lights on the left
pub mod style_macos {
    pub use ::style_macos::STYLE;
}

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
pub use self::style_winui as style;
#[cfg(jflib_style = "classic")]
pub use self::style_classic as style;
#[cfg(jflib_style = "kde")]
pub use self::style_kde as style;
#[cfg(jflib_style = "macos")]
pub use self::style_macos as style;
