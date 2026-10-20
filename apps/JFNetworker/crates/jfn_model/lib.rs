// jfn_model -- the data model JFNetworker's UI, simulator and command-line runner share:
// devices and their ports, cables, the project file and scenario scripts.  No I/O, no clock:
// the app reads and writes the text, the simulator (jfn_sim) runs what is described here.
//
//   catalog   Model (ports, CLI style, features, symbol), catalog(), model(id), DeviceKind,
//             Category, Media, PortSpec; speed_str
//   ifname    interface names the IOS way: "Gi1/0/1" <-> "GigabitEthernet1/0/1", abbreviations
//   cable     CableKind, cable_kind (the right cable for two ports), cable_fits (a chosen one),
//             link defaults (speed, delay), Why (German reasons)
//   project   Project / Device / Link / Note / Layout / ServiceDef records: add, rename, connect,
//             disconnect, remove and restore (undo), suggest_ports, check, to_text / from_text
//   scenario  .scn scripts for headless runs and tests: actions at virtual times and expectations
//   synth     topology(n): a synthetic campus of n devices with working configurations (tests
//             of large projects: --gen-topology, jfn-sim gen)
//
// Device identity is the project's numeric id; ports are referenced by their full interface
// name ("GigabitEthernet1/0/1", "eth0"), so files stay readable.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod cable;
pub mod catalog;
pub mod ifname;
pub mod project;
pub mod scenario;
pub mod synth;

pub use cable::{cable_fits, cable_kind, CableKind, Why};
pub use catalog::{catalog, model, Category, CliStyle, DeviceKind, Media, Model, PortSpec};
pub use project::{Device, Issue, Link, Note, PortRef, ProjErr, Project, ServiceDef};
pub use scenario::Scenario;
