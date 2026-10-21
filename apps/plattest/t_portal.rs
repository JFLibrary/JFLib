// t_portal -- (plat-shared) the xdg-desktop-portal calls against a session bus the caller
// provides (tools/platshared/portal_check.py runs a fake one; never a real dialog):
//   plattest --test portal --mode open --path P     plat::open_folder(P) (gfxtest --open of
//        the assembly: the exit code is 0 when it returned 0)
//   plattest --test portal --mode url --url U      plat::open_url(U)
//   plattest --test portal --mode filedlg [--flags F] [--path P] [--exts "png=PNG-Bild|jpg"]
//        [--expect R]                              plat::file_dialog: prints "filedlg: R
//        INDEX PATH"
// Linux only: on Windows these functions open real windows (win_shell.rs / win_filedlg.rs).
//                                                  [ASMLIB/src/plat/linux/portal.s]

use alloc::format;
use alloc::vec::Vec;

use crate::T;

pub fn t_portal(t: &mut T) {
    if cfg!(windows) {
        plat::write_stderr(b"plattest: --test portal is Linux only (the Windows calls open real windows)\n");
        plat::exit(2);
    }
    let arg = |n: &str| plat::arg_value(n);
    match arg("--mode").as_deref() {
        Some("open") => {
            let p = arg("--path").unwrap_or_default();
            let r = plat::open_folder(&p);
            crate::out(&format!("open: plat_open_folder({p}) -> {r}\n"));
            t.check(r == 0, "portal: plat_open_folder");
        }
        Some("url") => {
            let u = arg("--url").unwrap_or_default();
            let r = plat::open_url(&u);
            crate::out(&format!("url: plat_open_url({u}) -> {r}\n"));
            t.check(r == 0, "portal: plat_open_url");
        }
        Some("filedlg") => {
            let flags = arg("--flags").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
            let mut path = arg("--path").unwrap_or_default();
            let exts_s = arg("--exts").unwrap_or_default();
            let exts: Vec<&str> = if exts_s.is_empty() { Vec::new() } else { exts_s.split('|').collect() };
            let expect = arg("--expect").and_then(|v| v.parse::<i32>().ok()).unwrap_or(1);
            let (r, idx) = plat::file_dialog(flags, &mut path, &exts);
            crate::out(&format!("filedlg: {r} {idx} {path}\n"));
            t.check(r == expect, "portal: plat_file_dialog");
        }
        _ => {
            plat::write_stderr(b"usage: plattest --test portal --mode open|url|filedlg ...\n");
            plat::exit(2);
        }
    }
}
