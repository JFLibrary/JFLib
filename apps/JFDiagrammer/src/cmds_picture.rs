// cmds_picture -- (JD8 rest) Einfügen › Bilder › Bild aus Datei (RE-02, SP-09): a PNG, JPEG,
// GIF or BMP file as a picture shape
//
//   PictureState                the file dialog and its path
//   open(app, ui)               PICTURE_FILE: the file dialog (Bilder: png, jpg, jpeg, gif, bmp)
//   dialog(app, ui)             every pass: the dialog; a chosen file is inserted
//   insert_bytes(app, ui, bytes, centre) -> bool   a picture's bytes as a shape (also the
//                               clipboard's picture, cmds_paste.rs): one undo step
//                               "Bild einfügen", the shape selected
//
// The shape (diagram_core::ops::new_picture): Type Foreign, the bytes as a media part of the
// package (/visio/media/imageN.ext, written by diagram_io::vsdx::write), its size the picture's
// pixels at its resolution (96 dpi when the file says none; ops::probe_picture) in paper inches,
// scaled down to fit the page, in the middle of the canvas' view (Visio's place for an inserted
// picture).  A BMP is stored as PNG (lossless; Visio's own BMP handling in .vsdx is not
// documented), decoded and encoded through diagram_view.  Test aid: --picture-dir DIR is where
// the dialog starts (scripts; relative to the working folder).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{ops, Owner};
use ui::i18n::{t, tf};
use ui::*;

use crate::edit_canvas::key_of;
use crate::edit_doc;
use crate::Diagrammer;

const ID_DLG: u32 = 0x8000;

/// the file dialog of Bild aus Datei
pub(crate) struct PictureState {
    pub dlg: Shared<i32>,
    pub path: String,
}

impl Default for PictureState {
    fn default() -> PictureState {
        PictureState { dlg: Shared::new(0), path: String::new() }
    }
}

/// Bild aus Datei: the file dialog
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    if app.pictures.path.is_empty() {
        if let Some(d) = plat::arg_value("--picture-dir") {
            let d = if d.starts_with('/') || d.get(1..2) == Some(":") { d } else { format!("{}/{}", plat::cwd(), d) };
            app.pictures.path = format!("{}/", d.trim_end_matches(['/', '\\']));
        }
    }
    app.pictures.dlg.set(1);
    ui.repass();
}

/// every pass: the file dialog while it is open; a chosen file goes onto the page
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.pictures.dlg.clone();
    if dlg.get() == 0 {
        return;
    }
    let exts = [t("png;jpg;jpeg;gif;bmp=Bilder"), t("=Alle Dateien")];
    if ui.filedlg(ID_DLG, FD_OPEN, &dlg, &mut app.pictures.path, 4096, &exts) & EV_ACT != 0 {
        let path = ui.filedlg_paths().into_iter().next().unwrap_or_else(|| app.pictures.path.clone());
        insert_file(app, ui, &path);
    }
}

// a picture file onto the page
fn insert_file(app: &mut Diagrammer, ui: &mut Ui, path: &str) {
    let name = String::from(plat::display_name(path));
    let Some(bytes) = plat::read_file(path) else {
        app.error = Some((tf("„{}“ kann nicht eingefügt werden.", &[&name]), String::from(t("Die Datei lässt sich nicht lesen."))));
        ui.repass();
        return;
    };
    if !insert_bytes(app, ui, &bytes, None) {
        app.error = Some((tf("„{}“ kann nicht eingefügt werden.", &[&name]), String::from(t("Das Bild hat kein bekanntes Format (PNG, JPEG, GIF, BMP)."))));
        ui.repass();
    }
}

// a BMP as PNG: decoded and encoded again (None: not decodable)
fn bmp_as_png(bytes: &[u8], dpi: f64) -> Option<Vec<u8>> {
    let diagram_view::image::Decoded::Picture(p) = diagram_view::image::decode(bytes) else { return None };
    let mut px = Vec::with_capacity(p.px.len() * 4);
    for c in &p.px {
        px.extend_from_slice(&c.to_le_bytes());
    }
    let sf = gfx::Surface { px, w: p.w as i32, h: p.h as i32, stride: p.w as i32 * 4, fmt: gfx::FMT_ARGB, ..gfx::Surface::default() };
    diagram_view::export::raster::png(&sf, dpi)
}

/// a picture's bytes as a shape around `centre` (page point; None: the middle of the view),
/// one undo step, selected -> inserted (false: no PNG, JPEG, GIF or BMP, or no page)
pub(crate) fn insert_bytes(app: &mut Diagrammer, ui: &mut Ui, bytes: &[u8], centre: Option<[f64; 2]>) -> bool {
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return false };
    let Some(mut info) = ops::probe_picture(bytes) else { return false };
    let mut data = bytes.to_vec();
    if info.format == ops::PictureFormat::Bmp {
        let dpi = if info.dpi[0] >= 1.0 { info.dpi[0] } else { 96.0 };
        let Some(png) = bmp_as_png(bytes, dpi) else { return false };
        info.format = ops::PictureFormat::Png;
        data = png;
    }
    // the middle of the view, the page's size as the bound (drawing units)
    let vp = app.canvas.vp;
    let r = vp.rect;
    let mid = vp.to_world(r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
    let at = centre.unwrap_or_else(|| geo.page(mid));
    let per_inch = 1.0 / geo.scale;
    let size = info.size(per_inch, [geo.w * per_inch, geo.h * per_inch]);
    let format = info.format;
    let Some(o) = app.opened.as_mut() else { return false };
    let r = edit_doc::edit(o, t("Bild einfügen"), true, |tx, eng| ops::new_picture(tx, eng, Owner::Page(page), &data, format, at, size));
    match r {
        Ok(s) => {
            app.se.select(&[key_of(s)]);
            app.tool = crate::commands::POINTER;
            ui.repass();
            true
        }
        Err(_) => false,
    }
}
