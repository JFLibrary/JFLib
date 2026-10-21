// Data-only ELF fixtures: map and resolve a name, never call code from the file.
#![no_std]
#![no_main]
rt::main!({
    let args = plat::args();
    let Some(path) = args.get(1) else { return 2 };
    match plat::lib_open(path) {
        Ok(lib) => {
            plat::write_stdout(b"MAPPED\n");
            let symbol = plat::lib_sym(&lib, "review");
            if symbol == 0 { plat::write_stdout(b"NO_SYMBOL\n"); 1 }
            else { plat::write_stdout(b"SYMBOL_FOUND_NO_EXECUTION\n"); 0 }
        }
        Err(_) => { plat::write_stdout(b"REJECTED\n"); 2 }
    }
});
