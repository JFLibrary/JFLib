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
//   doc_begin / doc_image / doc_end / doc_abort   (print.rs PrintDoc, E9) PreparePrint once
//        (its connection stays open), the pages into one PDF of several pages (or the app's own
//        PDF: pdf_native), at the end the PDF in a memfd to Print; the test variants above
//        (jfgui_PRINT_TO writes the PDF of the pages into FILE).

use rt::os::{syscall6, SYS_CLOSE, SYS_LSEEK, SYS_MEMFD_CREATE, SYS_WRITE};

use crate::{
    ERR_CANCEL, ERR_DIMS, ERR_IO, ERR_UNSUPPORTED, PJF_LANDSCAPE, PJF_SMASK, Place, PrintDoc, PrintJob, PrintSrc, page_init,
    print_pdf, portal_print_end, portal_print_prepare, portal_print_send,
};

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
    let r = pdf_memfd(&pdf, fd);
    if r != 0 {
        return r;
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

// the PDF in a new memfd (fd: it, for the caller to close; -1 when none was made), read from
// its start -> 0 / ERR_IO
fn pdf_memfd(pdf: &[u8], fd: &mut isize) -> i32 {
    // SAFETY: memfd_create with a NUL-terminated name.
    *fd = unsafe { syscall6(SYS_MEMFD_CREATE, b"jfgui-print\0".as_ptr() as usize, MFD_CLOEXEC, 0, 0, 0, 0) };
    if *fd < 0 {
        return ERR_IO;
    }
    let mut rest = pdf;
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
    0
}

// ---------------------------------------------------------------- PrintDoc (E9)

/// (print.rs) PrintDoc::begin: the test variant or the portal's dialog (PreparePrint; its
/// connection stays open for doc_end) -> 0 / ERR_*
pub(crate) fn doc_begin(d: &mut PrintDoc) -> i32 {
    if crate::env("jfgui_PRINT_TO").is_some_and(|v| !v.is_empty()) {
        if env_is_1("jfgui_PRINT_SMASK") {
            d.flags |= PJF_SMASK; // test: alpha as /SMask instead of white
        }
        return crate::doc_begin_pdf_only(d);
    }
    if crate::disp_backend() == 0 && !env_is_1("jfgui_PRINT_PORTAL") {
        return ERR_UNSUPPORTED;
    }
    let job = PrintJob { flags: d.flags, paper: d.paper, ..PrintJob::new(None, &d.title) };
    let mut page = page_init(d.paper, d.flags);
    let r = portal_print_prepare(&job, &mut page, &d.title);
    if r <= 0 {
        portal_print_end();
        return if r < 0 { ERR_UNSUPPORTED } else { ERR_CANCEL };
    }
    // the dialog's orientation decides
    d.flags = (d.flags & !PJF_LANDSCAPE) | page.flags;
    d.page = page;
    d.copies = 1;
    d.sys.pdf = Some(Default::default());
    0
}

/// (print.rs) the pages go into the PDF here (pdf_native): never a page of its own
pub(crate) fn doc_image(_d: &mut PrintDoc, _src: &PrintSrc, _pl: &Place) -> i32 {
    ERR_UNSUPPORTED
}

/// (print.rs) PrintDoc::end: the test variant's file, or the PDF in a memfd to Print
pub(crate) fn doc_end(d: &mut PrintDoc) -> i32 {
    if d.sys.to.is_some() {
        return crate::doc_end_pdf_file(d);
    }
    let Some(pdf) = d.pdf_bytes() else {
        portal_print_end();
        return ERR_IO;
    };
    let mut fd: isize = -1;
    let mut r = pdf_memfd(&pdf, &mut fd);
    if r == 0 {
        r = match portal_print_send(&d.title, fd as i32, d.page.token) {
            1 => ERR_CANCEL,
            0 => 0,
            _ => ERR_IO,
        };
    }
    portal_print_end();
    if fd >= 0 {
        // SAFETY: our memfd.
        unsafe { syscall6(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0) };
    }
    r
}

/// (print.rs) PrintDoc::abort: the portal's connection closes
pub(crate) fn doc_abort(d: &mut PrintDoc) {
    if d.sys.to.is_none() {
        portal_print_end();
    }
}
