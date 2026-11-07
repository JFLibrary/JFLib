// hosted -- a program with the standard library (feature std, docs/CRATES.md CR6); the only file
// of plat that names std, compiled only with that feature.  Its start is std's (the C runtime's
// entry, std's main), so plat has no entry point of its own: the program's fn main calls
// std_start (win.rs, linux_glibc.rs), which sets up what plat's entry sets up without std.
// rt::main! and ui::main! write that fn main.
//
//   exit(code)          plat::exit: std::process::exit, which writes out what std's stdout holds
//   jfgui_std_start()   the symbol the fn main of rt::main! and ui::main! calls: std_start()
//   std_start()         (browser, Android) the host starts the app there (web.rs), never a fn main
//   initial_block()     (Linux) argc, argv[], 0, envp[], 0 from std, as the kernel lays them out

/// end the process: std::process::exit, which writes out what std's stdout still holds; from any
/// thread, not from a crash handler
pub fn exit(code: i32) -> ! {
    std::process::exit(code)
}

/// what rt::main!'s and ui::main!'s `fn main` calls: [`std_start`](crate::std_start)
#[unsafe(no_mangle)]
pub fn jfgui_std_start() -> ! {
    crate::std_start()
}

/// In the browser and on Android the host starts the app (web.rs), never a `fn main`.
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub fn std_start() -> ! {
    rt::os::write_stderr(b"plat::std_start: on this target the host starts the program, not its fn main\n");
    rt::os::exit(2)
}

/// argc, argv[], 0, envp[], 0 as the kernel lays them on the initial stack, from std's arguments
/// and environment (kept for the whole run): plat reads both from there (linux.rs)
#[cfg(target_os = "linux")]
pub(crate) fn initial_block() -> *const usize {
    use std::os::unix::ffi::OsStrExt;
    fn keep(text: &[u8]) -> usize {
        let mut z = text.to_vec();
        z.push(0);
        alloc::boxed::Box::leak(z.into_boxed_slice()).as_ptr() as usize
    }
    let args: alloc::vec::Vec<_> = std::env::args_os().collect();
    let mut block = alloc::vec![args.len()];
    block.extend(args.iter().map(|a| keep(a.as_bytes())));
    block.push(0);
    for (name, value) in std::env::vars_os() {
        let mut e = name.as_bytes().to_vec();
        e.push(b'=');
        e.extend_from_slice(value.as_bytes());
        block.push(keep(&e));
    }
    block.push(0);
    alloc::boxed::Box::leak(block.into_boxed_slice()).as_ptr()
}
