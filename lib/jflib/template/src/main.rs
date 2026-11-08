// The program.  The app is the library crate (src/lib.rs), so the same code also builds for the
// browser: cargo rustc --lib --crate-type cdylib --target wasm32-unknown-unknown.  Linking it
// brings ui::main!'s entry point.
#![no_std]
#![no_main]

extern crate hallo;
