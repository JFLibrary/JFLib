// android_print -- printing on Android: the job as our PDF (print.rs) to the system's PrintManager
// (android/GalleryPrint.java: a PrintDocumentAdapter that hands the PDF over; docs/ANDROID.md AN4)
//
//   print(job) -> 0 the system's print dialog has the job / ERR_UNSUPPORTED / ERR_DIMS / ERR_IO /
//        ERR_NOMEM.  The job laid out on its paper (job.paper, PJF_LANDSCAPE) as a one-page PDF
//        (print_pdf), handed to PrintManager.print.  The system's dialog -- a printer or "Save as
//        PDF", paper, copies, pages, with a preview of the PDF -- runs after print returns (the UI
//        thread must not wait for it: five seconds without an answer are an ANR), so 0 says the
//        dialog has the job, not that paper came out; job.copies = 1.  The job's paper and
//        orientation are the dialog's first choice; another paper scales the page (the print
//        spooler does that).
//   doc_begin / doc_image / doc_end / doc_abort   (print.rs PrintDoc, E9) no dialog before the
//        pages (the system's comes with the document, at end): page() is the job's paper, the pages
//        go into one PDF (pdf_native; add_pdf takes the app's own), end hands it over.
//   available() -> bool   the host has a print service and an activity (CALL_PRINT); a program
//        without the Java host (android/exe_main.c, the test APK) has none: ERR_UNSUPPORTED.
//   Test variant (never a dialog): jfgui_PRINT_TO=FILE writes the PDF into FILE, as on Linux
//        (jfgui_PRINT_SMASK=1: alpha as /SMask instead of white).
//
// REQUEST_PRINT (web.rs, detached): the title (u32 length + UTF-8), u32 paper (PAPER_*), u32
// flags (PJF_LANDSCAPE), u32 pages, then the PDF.  GalleryPrint takes it (the request stays
// pending) or refuses it at once (-3: no print service, no activity; the request is gone right
// after host_request, which print reads as ERR_UNSUPPORTED); later it completes with 2 (the
// system took the job), -4 (cancelled in the dialog) or a failure.

use alloc::vec::Vec;

use crate::web::{REQUEST_PRINT, RequestStatus, request, request_status};
use crate::{
    ERR_DIMS, ERR_IO, ERR_NOMEM, ERR_UNSUPPORTED, PJF_LANDSCAPE, PJF_SMASK, Place, PrintDoc, PrintJob, PrintSrc, page_init,
    print_pdf,
};

/// (GalleryPrint.java) -> u32 1 when the system has a print service and the activity is there
const CALL_PRINT: u32 = 70;

fn env_is_1(name: &str) -> bool {
    crate::env(name).is_some_and(|v| v.as_bytes().first() == Some(&b'1'))
}

fn test_file() -> Option<alloc::string::String> {
    crate::env("jfgui_PRINT_TO").filter(|v| !v.is_empty())
}

/// whether printing goes anywhere here: the host's print service (not the test variant)
pub(crate) fn available() -> bool {
    crate::web::android_host::call(CALL_PRINT, &[]).is_some_and(|r| r.first() == Some(&1))
}

// the PDF to the host's PrintManager -> 0 the dialog has it / ERR_UNSUPPORTED / ERR_NOMEM / ERR_IO
fn hand_over(title: &str, paper: u32, flags: u32, pages: u32, pdf: &[u8]) -> i32 {
    let mut b = Vec::new();
    if b.try_reserve_exact(16 + title.len() + pdf.len()).is_err() {
        return ERR_NOMEM;
    }
    b.extend_from_slice(&(title.len() as u32).to_le_bytes());
    b.extend_from_slice(title.as_bytes());
    for v in [paper, flags & PJF_LANDSCAPE, pages] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(pdf);
    let Some(id) = request(REQUEST_PRINT, &b, None) else { return ERR_IO };
    // detached: an answer at once (the host's refusal) took the request out already
    match request_status(id) {
        Some(RequestStatus::Pending) => 0,
        _ => ERR_UNSUPPORTED,
    }
}

/// print one image on one page (see the file header)  [plat_print]
pub fn print(job: &mut PrintJob<'_>) -> i32 {
    job.copies = 0;
    if !crate::policy::print_allowed() {
        return ERR_UNSUPPORTED;
    }
    if job.src.is_none() {
        return ERR_DIMS;
    }
    let to = test_file();
    if to.is_some() && env_is_1("jfgui_PRINT_SMASK") {
        job.flags |= PJF_SMASK; // test: alpha as /SMask instead of white
    }
    let page = page_init(job.paper, job.flags);
    let pdf = match print_pdf(job, &page) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let r = match to {
        Some(to) => {
            if crate::write_file_raw(&to, &pdf) { 0 } else { ERR_IO }
        }
        None => {
            let title = if job.title.is_empty() || job.title.starts_with('\0') { "Bild" } else { job.title };
            hand_over(title, job.paper, job.flags, 1, &pdf)
        }
    };
    if r == 0 {
        job.copies = 1;
    }
    r
}

/// (print.rs) PrintDoc::begin: the test variant, or the PDF of the pages for the host's print
/// service (its dialog comes at end) -> 0 / ERR_UNSUPPORTED (no print service)
pub(crate) fn doc_begin(d: &mut PrintDoc) -> i32 {
    if test_file().is_some() {
        if env_is_1("jfgui_PRINT_SMASK") {
            d.flags |= PJF_SMASK; // test: alpha as /SMask instead of white
        }
        return crate::doc_begin_pdf_only(d);
    }
    if !available() {
        return ERR_UNSUPPORTED;
    }
    d.copies = 1;
    d.sys.pdf = Some(Default::default());
    0
}

/// (print.rs) the pages go into the PDF here (pdf_native): never a page of its own
pub(crate) fn doc_image(_d: &mut PrintDoc, _src: &PrintSrc, _pl: &Place) -> i32 {
    ERR_UNSUPPORTED
}

/// (print.rs) PrintDoc::end: the test variant's file, or the PDF to the system's print dialog
pub(crate) fn doc_end(d: &mut PrintDoc) -> i32 {
    if d.sys.to.is_some() {
        return crate::doc_end_pdf_file(d);
    }
    let Some(pdf) = d.pdf_bytes() else { return ERR_IO };
    // the pages added (the app's own PDF: unknown, 0)
    let pages = if d.sys.raw.is_some() { 0 } else { d.added };
    hand_over(&d.title, d.paper, d.flags, pages, &pdf)
}

/// (print.rs) PrintDoc::abort: nothing went to the host yet
pub(crate) fn doc_abort(_d: &mut PrintDoc) {}
