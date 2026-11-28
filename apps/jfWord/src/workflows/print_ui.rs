// Datei / Drucken: the pages exactly as the preview lays them out, one picture each, through the
// system's print dialog (plat::PrintDoc: Windows' dialog, Linux's portal, Android's PrintManager,
// whose dialog comes after the pages, with "Save as PDF").  No own dialog, no vector PDF yet.
use crate::Word;
use alloc::{format, rc::Rc, string::String};
use doc_view::PageDraw;
use ui::*;

/// the pictures' resolution; PageDraw::thumbnail allows 2048 x 4096 pixels (an A4 page: 1654 x 2339)
const PRINT_DPI: u64 = 200;
const TWIPS_PER_INCH: u64 = 1440;

/// the paper of a page (twips, shorter and longer side) the dialog suggests: the nearest PAPER_*
fn paper_of(short: u32, long: u32) -> u32 {
    // PAPER_* portrait sizes in twips
    const SIZES: [(u32, u32, u32); 5] = [
        (plat::PAPER_A4, 11906, 16838),
        (plat::PAPER_LETTER, 12240, 15840),
        (plat::PAPER_A5, 8391, 11906),
        (plat::PAPER_LEGAL, 12240, 20160),
        (plat::PAPER_A3, 16838, 23811),
    ];
    SIZES
        .iter()
        .min_by_key(|(_, w, h)| w.abs_diff(short) + h.abs_diff(long))
        .map_or(plat::PAPER_A4, |s| s.0)
}

/// printing is there (CAP_PRINT: the platform prints and PERMISSIONS allows it) for a document of
/// the own model (the preserved DOCX view shows a text excerpt, no pages)
pub(crate) fn print_available(word: &Word) -> bool {
    word.shell.has_document
        && !word.preserved()
        && plat::capabilities().supports(plat::CAP_PRINT)
}

impl Word {
    pub(crate) fn print_document(&mut self, ui: &mut Ui) {
        self.message = match self.print_pages(ui) {
            Ok(n) => {
                self.shell.home = false; // back to the document, as after saving
                if n == 1 { String::from("Druckauftrag mit 1 Seite übergeben") } else { format!("Druckauftrag mit {n} Seiten übergeben") }
            }
            Err(message) => message,
        };
    }

    // -> the pages handed over (Android: to its print dialog, which prints or saves them later)
    fn print_pages(&mut self, ui: &mut Ui) -> Result<u32, String> {
        let fonts = self
            .preview
            .prepare_fonts(&mut ui.text)
            .map_err(|e| format!("Dokumentschrift nicht verfügbar: {e:?}"))?;
        let layout = doc_view::paginate(self.editor.document(), &mut ui.text, &fonts)
            .map_err(|e| format!("Seiten nicht darstellbar: {e:?}"))?;
        let layout = Rc::new(layout);
        let pages = layout.pages.len();
        let (w, h) = (layout.settings.width, layout.settings.height);
        if pages == 0 || w == 0 || h == 0 {
            return Err(String::from("Das Dokument hat keine Seite zum Drucken."));
        }
        let title = if self.path_name.is_empty() { "Dokument" } else { self.path_name.as_str() };
        let mut job = plat::PrintJob::new(None, title);
        job.paper = paper_of(w.min(h), w.max(h));
        // fitted into the printable area, centred: on paper of the document's size (a PDF)
        // exactly its size, on a printer with unprintable edges slightly smaller
        job.flags = plat::PJF_CENTER | if w > h { plat::PJF_LANDSCAPE } else { 0 };
        job.scale = plat::PJS_FIT;
        let mut doc = match plat::PrintDoc::begin(&job, pages as u32) {
            Ok(doc) => doc,
            Err(plat::ERR_CANCEL) => return Err(String::from("Drucken abgebrochen.")),
            Err(plat::ERR_UNSUPPORTED) => {
                return Err(String::from("Drucken ist auf diesem System nicht möglich."))
            }
            Err(e) => return Err(format!("Drucken fehlgeschlagen (Fehler {e}).")),
        };
        let mut width = ((u64::from(w) * PRINT_DPI / TWIPS_PER_INCH) as i32).clamp(1, 2048);
        let height = doc_view::page_height(w, h, width);
        if height > 4096 {
            width = (i64::from(width) * 4096 / i64::from(height)).max(1) as i32;
        }
        let mut printed = 0;
        for index in 0..pages {
            if !doc.wants(index as u32 + 1) {
                continue;
            }
            let picture = PageDraw::new(layout.clone(), index, fonts.clone())
                .and_then(|page| page.without_frame().thumbnail(&mut ui.text, width));
            let surface = match picture {
                Ok(surface) => surface,
                Err(e) => {
                    doc.abort();
                    return Err(format!("Seite {} nicht darstellbar: {e:?}", index + 1));
                }
            };
            let src = plat::PrintSrc {
                px: &surface.px,
                w: surface.w,
                h: surface.h,
                stride: surface.stride,
                fmt: plat::PRINT_PARGB,
            };
            let r = doc.add_image(&src);
            if r != 0 {
                doc.abort();
                return Err(format!("Drucken fehlgeschlagen (Fehler {r})."));
            }
            printed += 1;
        }
        if printed == 0 {
            doc.abort();
            return Err(String::from("Im gewählten Bereich liegt keine Seite."));
        }
        match doc.end() {
            0 => Ok(printed),
            plat::ERR_CANCEL => Err(String::from("Drucken abgebrochen.")),
            e => Err(format!("Drucken fehlgeschlagen (Fehler {e}).")),
        }
    }
}

