// jfn_model -- the data model JFNetworker's UI, simulator and command-line runner share:
// devices and their ports, cables, the project file and scenario scripts.  No I/O, no clock:
// the app reads and writes the text, the simulator (jfn_sim) runs what is described here.
//
//   catalog   Model (ports, CLI style, features, symbol), catalog(), model(id), DeviceKind,
//             Category, Media, PortSpec; speed_str / speed_msg, Model::summary_msg
//   ifname    interface names the IOS way: "Gi1/0/1" <-> "GigabitEthernet1/0/1", abbreviations
//   cable     CableKind, cable_kind (the right cable for two ports), cable_fits (a chosen one),
//             link defaults (speed, delay), Why (German reasons, Why::msg)
//   project   Project / Device / Link / Note / Layout / ServiceDef records: add, rename, connect,
//             disconnect, remove and restore (undo), suggest_ports, check, to_text / from_text
//   scenario  .scn scripts for headless runs and tests: actions at virtual times and expectations
//   msg       Msg and msg!: the German texts the app shows as templates with arguments, so its
//             catalogue (src/locales/engine.tsv) can translate them (text / text_with)
//   synth     topology(n): a synthetic campus of n devices with working configurations (tests
//             of large projects: --gen-topology, jfn-sim gen)
//   lab       Lab, Goal, Text: the task of a lab project (title, introduction, goals with checks
//             in the expectation syntax of scenarios, hints; German and English)
//             Assess, Item, Target (A1): an assessment of a lab -- graded items with points,
//             the target network, the feedback; canon / RouteVal: the canonical item values
//   wlan      Wlan, WlanNet, WlanJoin, Band, Security: the networks of access points and what
//             stations join; bands, ranges, radio ports, channels of the models; the scale
//             of the canvas (dip_per_m)
//   files     File (a device's file in the project), Store (flash: and nvram: of IOS-like
//             devices, the Linux tree of hosts: paths, read, write, list, limits), capacity_of /
//             image_of (the size limit and system image of a model)
//   phys      Place, PlaceKind, Spot, Tree (P1): the places of the physical view -- sites,
//             buildings, floors, rooms, racks, desks --, where the devices stand, the length of
//             every cable (by hand or automatic) and the reach of its medium (IEEE 802.3)
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
pub mod msg;
pub mod synth;
pub mod lab;
pub mod wlan;
pub mod files;
pub mod phys;

pub use cable::{cable_fits, cable_kind, CableKind, Why};
pub use catalog::{catalog, model, Category, CliStyle, DeviceKind, Media, Model, PortSpec};
pub use project::{Device, Issue, Link, Note, PortRef, ProjErr, Project, ServiceDef};
pub use scenario::Scenario;
pub use msg::Msg;
pub use lab::{Goal, Lab, Text};
pub use lab::{Assess, Compare, Feedback, Item, ItemKind, Target};
pub use wlan::{Band, Security, Wlan, WlanJoin, WlanNet};
pub use catalog::{Form, Optic};
pub use phys::{Len, Place, PlaceKind, Reach, Spot, Tree};
