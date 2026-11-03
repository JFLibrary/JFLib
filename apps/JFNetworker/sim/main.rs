// jfn-sim -- JFNetworker without a window: scenarios against projects, checks of projects and
// their startup configurations, frames decoded from hex (the logic is in crates/jfn_run)
//
//   jfn-sim run PROJEKT [--scenario S] [--pcap F] [--report F] [--golden F]
//   jfn-sim check PROJEKT
//   jfn-sim course ORDNER     a course's folder: its file, lessons, pictures, links, labs (AK1)
//   jfn-sim course-install QUELLE KURS|all --into ORDNER   a course from an address or a folder (AK3)
//   jfn-sim course-list QUELLE [--into ORDNER] / course-remove KURS --into ORDNER
//   jfn-sim decode HEX
//   jfn-sim gen N [--out F]   a synthetic campus of N devices (jfn_model::synth)
//   jfn-sim rewind P [S] [--traffic K]   keyframes of a run and restore times (K1)
//   jfn-sim sweep F [--threads N] [--csv C]   an experiment at full speed (SA2)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

fn run() -> i32 {
    jfn_run::main(&plat::args())
}

rt::main!(run());
