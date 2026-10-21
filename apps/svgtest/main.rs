// svgtest -- SVG images (lib/img_svg) and the image viewer (ui.image_view); for tests/*.uis
//
//   build/win/svgtest.exe --test unit     self-checks of img_svg (unit.rs); exit code = failures
//   build/win/svgtest.exe --script apps/svgtest/tests/gallery.uis
//   build/win/svgtest.exe                 a window
//
// Tab "Galerie": every fixture of apps/svgtest/fixtures (self-written SVGs: shapes, paths,
// strokes, gradients, transforms, viewBox, opacity, clipping, CSS, use / symbol, text, an
// embedded PNG, an editor's file) through ui.svg at 180 DIP, then the icon fixture at 16, 24,
// 32, 48 and 96 DIP in the text colour.  Tab "Betrachter": ui.image_view (id 500) on a 32 x 24
// pixel image, a 1600 x 1200 "photo" (rings: aliasing shows when shrinking is wrong) or an
// SVG; the status line shows zoom, centre and fit mode.  "Hoher Kontrast" switches the theme.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod unit;

use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;

use gfx::{Surface, FMT_PARGB};
use ui::*;

const DESC: AppDesc = AppDesc { title: "jfgui SVG", flags: AF_RESIZABLE, w: 1040, h: 760, min_w: 480, min_h: 360, ..AppDesc::NONE };

/// the fixtures (name, file)
pub(crate) const FIXTURES: [(&str, &[u8]); 13] = [
    ("shapes", include_bytes!("fixtures/shapes.svg")),
    ("paths", include_bytes!("fixtures/paths.svg")),
    ("strokes", include_bytes!("fixtures/strokes.svg")),
    ("gradients", include_bytes!("fixtures/gradients.svg")),
    ("transforms", include_bytes!("fixtures/transforms.svg")),
    ("viewbox", include_bytes!("fixtures/viewbox.svg")),
    ("opacity", include_bytes!("fixtures/opacity.svg")),
    ("clip", include_bytes!("fixtures/clip.svg")),
    ("css", include_bytes!("fixtures/css.svg")),
    ("use", include_bytes!("fixtures/use.svg")),
    ("text", include_bytes!("fixtures/text.svg")),
    ("image", include_bytes!("fixtures/image.svg")),
    ("editor", include_bytes!("fixtures/editor.svg")),
];
static ICON: &[u8] = include_bytes!("fixtures/icon.svg");

const ID_TABS: u32 = 100;
const ID_HC: u32 = 110;
const ID_PICK: u32 = 120;
const ID_FIT: u32 = 130;
const ID_ACTUAL: u32 = 131;
const ID_IN: u32 = 132;
const ID_OUT: u32 = 133;
const ID_SMOOTH: u32 = 134;
const ID_SVG: u32 = 200; // + fixture index
const ID_ICON: u32 = 300; // + size index
const ID_VIEW: u32 = 500;
const ICON_SIZES: [i32; 5] = [16, 24, 32, 48, 96];

struct SvgTest {
    tab: i32,
    hc: i32,
    docs: Vec<(&'static str, Rc<Svg>)>,
    icon: Option<Rc<Svg>>,
    pixel: Rc<Surface>,
    photo: Rc<Surface>,
    pick: i32,
    smooth: i32,
    st: ImageViewState,
}

// a 32 x 24 test picture: colour blocks, a fading row, a transparent corner, a diagonal
fn pixel_image() -> Surface {
    let (w, h) = (32, 24);
    let mut sf = Surface::new(w, h, FMT_PARGB);
    let cols = [0xE53935u32, 0x43A047, 0x1E88E5, 0xFDD835];
    for y in 0..h {
        for x in 0..w {
            let mut c = 0xFF00_0000 | cols[((x / 8 + y / 6) % 4) as usize];
            if y >= 18 {
                // a row fading out: alpha 255 .. 8
                let a = (255 - x * 8) as u32;
                c = a << 24 | 0x6A1B9A;
            }
            if x == y {
                c = 0xFFFF_FFFF;
            }
            if x < 4 && y < 4 {
                c = 0;
            }
            // premultiply
            let a = c >> 24;
            let mut p = a << 24;
            for k in 0..3 {
                p |= ((c >> (k * 8)) & 255) * a / 255 << (k * 8);
            }
            let o = y as usize * sf.stride as usize + x as usize * 4;
            sf.px[o..o + 4].copy_from_slice(&p.to_le_bytes());
        }
    }
    sf
}

// a 1600 x 1200 "photo": colour ramps under fine rings (a zone plate of integers)
fn photo_image() -> Surface {
    let (w, h) = (1600, 1200);
    let mut sf = Surface::new(w, h, FMT_PARGB);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x - w / 2, y - h / 2);
            let r2 = (dx * dx + dy * dy) as u32;
            let ring = (r2 / 1400) & 1 == 0;
            let r = (x * 255 / w) as u32;
            let g = (y * 255 / h) as u32;
            let b = 160u32;
            let (r, g, b) = if ring { (r, g, b) } else { (r / 3, g / 3, b / 3) };
            let p = 0xFF00_0000 | r << 16 | g << 8 | b;
            let o = y as usize * sf.stride as usize + x as usize * 4;
            sf.px[o..o + 4].copy_from_slice(&p.to_le_bytes());
        }
    }
    sf
}

impl SvgTest {
    fn new() -> SvgTest {
        let docs = FIXTURES.iter().filter_map(|(n, d)| Svg::parse(d).ok().map(|s| (*n, Rc::new(s)))).collect();
        SvgTest {
            tab: 0,
            hc: 0,
            docs,
            icon: Svg::parse(ICON).ok().map(Rc::new),
            pixel: Rc::new(pixel_image()),
            photo: Rc::new(photo_image()),
            pick: 0,
            smooth: 0,
            st: ImageViewState::default(),
        }
    }

    fn gallery(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.scroll_begin(wid("gallery"), SC_V);
        ui.box_begin(0, BOX_V | pad(4), 12);
        for row in self.docs.chunks(5).enumerate() {
            ui.box_begin(0, BOX_H, 12);
            for (k, (name, doc)) in row.1.iter().enumerate() {
                let i = row.0 * 5 + k;
                ui.box_begin(0, BOX_V | BOX_XCENTER, 4);
                ui.svg(ID_SVG + i as u32, doc, 180, 180);
                ui.text(TS_CAPTION | TX_SECONDARY, name);
                ui.box_end();
            }
            ui.box_end();
        }
        if let Some(icon) = &self.icon {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
            ui.text(TS_BODY, "Symbol in der Textfarbe:");
            for (k, &s) in ICON_SIZES.iter().enumerate() {
                ui.svg(ID_ICON + k as u32, icon, s, s);
            }
            ui.box_end();
        }
        ui.box_end();
        ui.scroll_end();
    }

    fn viewer(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.tabs(ID_PICK, &mut self.pick, &["Pixel", "Foto", "SVG"]);
        ui.spacer(-1);
        if ui.button(ID_FIT, "Einpassen") & EV_ACT != 0 {
            self.st.set_fit();
        }
        if ui.button(ID_ACTUAL, "100 %") & EV_ACT != 0 {
            self.st.set_actual();
        }
        if ui.button(ID_IN, "Größer") & EV_ACT != 0 {
            self.st.zoom_by(2.0);
        }
        if ui.button(ID_OUT, "Kleiner") & EV_ACT != 0 {
            self.st.zoom_by(0.5);
        }
        ui.checkbox(ID_SMOOTH, "Glatt", &mut self.smooth);
        ui.box_end();
        let st = &self.st;
        let line = format!("zoom: {} %  cx: {:.1}  cy: {:.1}  fit: {}", st.zoom_pct(), st.cx, st.cy, st.fit as i32);
        ui.text(TS_BODY, &line);
        let flags = if self.smooth != 0 { IV_SMOOTH } else { 0 };
        let svg = self.docs.iter().find(|d| d.0 == "paths").map(|d| d.1.clone());
        match (self.pick, svg) {
            (0, _) => {
                ui.image_view(ID_VIEW, &mut self.st, ViewImage::Bitmap(&self.pixel, 1), flags);
            }
            (1, _) => {
                ui.image_view(ID_VIEW, &mut self.st, ViewImage::Bitmap(&self.photo, 1), flags);
            }
            (_, Some(d)) => {
                ui.image_view(ID_VIEW, &mut self.st, ViewImage::Svg(&d), flags);
            }
            _ => {}
        }
    }
}

impl App for SvgTest {
    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | pad(12), 8);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.tabs(ID_TABS, &mut self.tab, &["Galerie", "Betrachter"]);
        ui.spacer(-1);
        ui.next_flags(ST_NOFOCUS);
        if ui.checkbox(ID_HC, "Hoher Kontrast", &mut self.hc) & EV_ACT != 0 {
            ui.set_contrast(Some(self.hc != 0));
        }
        ui.box_end();
        if self.tab == 0 {
            self.gallery(ui);
        } else {
            self.viewer(ui);
        }
        ui.box_end();
    }
}

fn svgtest() -> i32 {
    if plat::arg_value("--test").as_deref() == Some("unit") {
        return unit::run();
    }
    ui::launch(&DESC, SvgTest::new(), &style::STYLE)
}

ui::main!(svgtest());
