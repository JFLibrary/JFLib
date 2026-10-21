// JFNetworker -- network simulator and lab for distributed systems (an alternative to Cisco
// Packet Tracer): devices, cables, real protocols byte for byte, and services to build scaled
// systems with.
//
// The window is the topology editor: a device palette, the canvas with the device symbols,
// the inspector, the (still empty) panels of the simulator, .jfnet projects with undo.  The
// data model lives in crates/jfn_model, the simulator in crates/jfn_sim.
//
//   python build_rs.py app JFNetworker
//   build/win/JFNetworker.exe                     the editor
//   build/win/JFNetworker.exe --open FILE.jfnet   ... with a project
//   build/win/JFNetworker.exe --selftest          self-checks of model and simulator (tests/model.uis)
//
// Modules: app/ (start, state, commands, ids), workspace/ (shell, palette, inspector, canvas,
// icons, the Terminal and Protokoll panels, device windows), doc/ (files, undoable edits, the
// simulator and the consoles); selftest.rs, simtest.rs and consoletest.rs check jfn_model,
// jfn_sim and what the consoles build on.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod app;
mod consoletest;
mod doc;
mod selftest;
#[cfg(any(windows, target_os = "linux"))]
mod sim_thread;
mod simtest;
mod workspace;

ui::main!(app::run());
