//! PaintCraft raster editor. See ../docs/ARCHITECTURE.md for module ownership.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![allow(dead_code)]

extern crate alloc;

mod app;
mod diagnostics;
mod document;
mod effects;
mod integrations;
mod preferences;
mod tools;
mod workspace;

ui::main!(app::run());
