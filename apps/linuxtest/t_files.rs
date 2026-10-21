// t_files -- linuxtest: files, environment, time and the crash handler
//                                                    [apps/linuxtest/src/main.s: test_files, test_crash]

use alloc::format;

use crate::{Lt, out};

const TEXT: &str = "Hello Größe\n"; // 14 bytes

/// file read / write / exists / dir_list, getenv, theme, sleep / time
pub fn test_files(t: &mut Lt) {
    let ok = plat::write_file("build/lt_file.txt", TEXT.as_bytes());
    t.check(ok, "file_write");
    let data = plat::read_file("build/lt_file.txt");
    t.check(data.is_some(), "file_read");
    if let Some(d) = data {
        t.check(d.len() == 14, "file_read length 14");
        t.check(d == TEXT.as_bytes(), "file_read content");
    }
    t.check(plat::file_exists("build/lt_file.txt") == 1, "file_exists file");
    t.check(plat::file_exists("build") == 2, "file_exists dir");
    t.check(plat::file_exists("build/does-not-exist") == 0, "file_exists missing");
    t.check(plat::read_file("build/does-not-exist").is_none(), "file_read missing -> 0");
    t.check(plat::read_file("build").is_none(), "file_read directory -> 0");
    #[cfg(target_os = "linux")]
    {
        let p = plat::read_file("/proc/self/stat");
        t.check(p.is_some_and(|d| d.len() > 20), "file_read /proc file (size 0 in stat)");
    }
    t.check(!plat::write_file("build/no/such/dir/x.txt", b"x"), "file_write bad path -> -1");
    #[cfg(target_os = "linux")]
    {
        let mut found = 0u32;
        let ok = plat::dir_list("build", &mut |name, is_dir, size| {
            out(&format!("  entry {} dir={} size={}\n", name, is_dir as u32, size));
            if name == "lt_file.txt" && !is_dir && size == 14 {
                found += 1;
            }
        });
        t.check(ok, "dir_list");
        t.check(found == 1, "dir_list saw lt_file.txt (14 bytes, file)");
        t.check(!plat::dir_list("build/does-not-exist", &mut |_, _, _| {}), "dir_list missing -> -1");
    }
    out(&format!("getenv HOME = {}\n", plat::getenv("HOME").as_deref().unwrap_or("(null)")));
    out(&format!("plat_theme = {}\n", plat::theme()));
    let t0 = plat::time_us();
    plat::sleep_ms(50);
    let us = plat::time_us() - t0;
    t.check((49000..200000).contains(&us), "sleep_ms(50) / time_us");
    out(&format!("slept {us} us\n"));
}

/// a write to a null page -> the crash handler's stderr line, exit code 3 (the assembly
/// writes to address 0, plat::crash_test to 0x10: apps have no unsafe code)
pub fn test_crash(_t: &mut Lt) {
    plat::crash_test();
}
