// hosted -- what rt has for a program with the standard library (feature std, docs/CRATES.md
// CR6); the only file of rt that names std, compiled only with that feature.  std brings the panic
// handler, the allocator and the memory functions then (lib.rs, mem.rs and os.rs leave theirs out).
//
//   panic_hook()   std's panics end as rt's panic handler ends them without std

/// (feature std) std's panics end as rt's panic handler ends them without std: one line on stderr,
/// the crash hook, exit code 3.  plat's start of a std program installs it; replacing it with
/// `std::panic::set_hook` gives up JFLib's crash report for panics.
pub fn panic_hook() {
    std::panic::set_hook(alloc::boxed::Box::new(|info| crate::panic_report(info)));
}
