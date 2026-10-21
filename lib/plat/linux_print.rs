// linux_print -- printing on Linux: xdg-desktop-portal Print with a PDF of the job
//                                                    [src/plat/linux/lx_print.s; docs/modules/PRINT.md]
//
//   print(job) -> 0 printed / ERR_CANCEL / ERR_UNSUPPORTED / ERR_IO / ERR_NOMEM / ERR_DIMS.
//        PreparePrint (the portal's dialog: printer, paper, copies; our paper and orientation
//        suggested), then the job laid out on the chosen page as a one-page PDF (print_pdf)
//        in a memfd handed to Print.
//   Test / headless variants (never a dialog):
//        jfgui_PRINT_TO=FILE.pdf    the PDF is written to FILE (paper from job.paper)
//        jfgui_PRINT_SMASK=1        (with jfgui_PRINT_TO) alpha as /SMask instead of white
//        jfgui_PRINT_PORTAL=1       allow the portal without a window (fake bus tests);
//        otherwise a run without a display (headless) returns ERR_UNSUPPORTED.

use rt::os::syscall6;

use crate::{
    ERR_CANCEL, ERR_DIMS, ERR_IO, ERR_UNSUPPORTED, PJF_LANDSCAPE, PJF_SMASK, PrintJob, page_init, print_pdf,
    portal_print_end, portal_print_prepare, portal_print_send,
};

const SYS_WRITE: usize = 1;
const SYS_CLOSE: usize = 3;
const SYS_LSEEK: usize = 8;
const SYS_MEMFD_CREATE: usize = 319;
const MFD_CLOEXEC: usize = 1;
const EINTR: isize = -4;

fn env_is_1(name: &str) -> bool {
    crate::env(name).is_some_and(|v| v.as_bytes().first() == Some(&b'1'))
}

/// print one image on one page (see the file header)  [plat_print]
pub fn print(job: &mut PrintJob) -> i32 {
    job.copies = 0;
    if !crate::policy::print_allowed() {
        return ERR_UNSUPPORTED;
    }
    if job.src.is_none() {
        return ERR_DIMS;
    }
    let title = if job.title.is_empty() || job.title.starts_with('\0') { "Bild" } else { job.title };
    let mut page = page_init(job.paper, job.flags);
    // ---- test variant: straight into a file
    if let Some(to) = crate::env("jfgui_PRINT_TO").filter(|v| !v.is_empty()) {
        if env_is_1("jfgui_PRINT_SMASK") {
            job.flags |= PJF_SMASK; // test: alpha as /SMask instead of white
        }
        let pdf = match print_pdf(job, &page) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if !crate::write_file_raw(&to, &pdf) {
            return ERR_IO;
        }
        job.copies = 1;
        return 0;
    }
    // ---- no window: only with jfgui_PRINT_PORTAL=1 (never a real dialog in tests)
    if crate::disp_backend() == 0 && !env_is_1("jfgui_PRINT_PORTAL") {
        return ERR_UNSUPPORTED;
    }
    let mut fd: isize = -1;
    let r = print_portal(job, &mut page, title, &mut fd);
    portal_print_end();
    if fd >= 0 {
        // SAFETY: our memfd.
        unsafe { syscall6(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0) };
    }
    r
}

fn print_portal(job: &mut PrintJob, page: &mut crate::PPage, title: &str, fd: &mut isize) -> i32 {
    let r = portal_print_prepare(job, page, title);
    if r < 0 {
        return ERR_UNSUPPORTED;
    }
    if r == 0 {
        return ERR_CANCEL;
    }
    // the dialog's orientation decides
    job.flags = (job.flags & !PJF_LANDSCAPE) | page.flags;
    let pdf = match print_pdf(job, page) {
        Ok(p) => p,
        Err(e) => return e,
    };
    // the PDF in a memfd
    // SAFETY: memfd_create with a NUL-terminated name.
    *fd = unsafe { syscall6(SYS_MEMFD_CREATE, b"jfgui-print\0".as_ptr() as usize, MFD_CLOEXEC, 0, 0, 0, 0) };
    if *fd < 0 {
        return ERR_IO;
    }
    let mut rest = &pdf[..];
    while !rest.is_empty() {
        // SAFETY: write from a live slice.
        let n = unsafe { syscall6(SYS_WRITE, *fd as usize, rest.as_ptr() as usize, rest.len(), 0, 0, 0) };
        if n == EINTR {
            continue;
        }
        if n <= 0 {
            return ERR_IO;
        }
        rest = &rest[n as usize..];
    }
    // SAFETY: lseek(fd, 0, SEEK_SET).
    if unsafe { syscall6(SYS_LSEEK, *fd as usize, 0, 0, 0, 0, 0) } != 0 {
        return ERR_IO;
    }
    match portal_print_send(title, *fd as i32, page.token) {
        1 => ERR_CANCEL,
        0 => {
            job.copies = 1;
            0
        }
        _ => ERR_IO,
    }
}
