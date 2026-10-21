// jfn-sim -- JFNetworker without a window: scenarios against projects, checks of projects and
// their startup configurations, frames decoded from hex (the logic is in crates/jfn_run)
//
//   jfn-sim run PROJEKT [--scenario S] [--pcap F] [--report F] [--golden F]
//   jfn-sim check PROJEKT
//   jfn-sim decode HEX
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

fn run() -> i32 {
    jfn_run::main(&plat::args())
}

rt::main!(run());
