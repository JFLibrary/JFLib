// "Bild & Leinwand": an image viewer that pans and zooms from tiles on the GPU (ui.image_view,
// canvas.rs), and below it a big opaque picture that becomes a texture once its redraws have cost
// as much as its upload (ui.image, docs/GPU.md "Ebenen und Bilder").  Without the GPU output the
// same page, drawn on the CPU; android_gpu.py and tests/pages.uis go through it.

use alloc::rc::Rc;

use gfx::{Surface, FMT_XRGB};
use ui::*;
use ui::i18n::t;

const ID_VIEW: u32 = 9000;
const VIEW_SIZE: i32 = 1600;
const PICTURE_W: i32 = 960;
const PICTURE_H: i32 = 1800;

pub struct Images {
    view: ImageViewState,
    made: Option<(Rc<Surface>, Rc<Surface>)>,
}

impl Default for Images {
    fn default() -> Self {
        // at 100 %: larger than the view, it can be moved at once
        let mut view = ImageViewState::default();
        view.fit = false;
        view.zoom = 1.0;
        view.cx = VIEW_SIZE as f64 / 2.0;
        view.cy = VIEW_SIZE as f64 / 2.0;
        Self { view, made: None }
    }
}

// something with edges everywhere, the same on every target
fn pattern(w: i32, h: i32, salt: u32) -> Rc<Surface> {
    let mut sf = Surface::new(w, h, FMT_XRGB);
    let stride = sf.stride as usize;
    for y in 0..h as u32 {
        let row = &mut sf.px[y as usize * stride..][..w as usize * 4];
        for (x, p) in row.chunks_exact_mut(4).enumerate() {
            let x = x as u32;
            let cell = (x / 64 + y / 64 + salt) & 1;
            let r = (x * 255 / w as u32) as u8;
            let g = (y * 255 / h as u32) as u8;
            let b = if cell == 0 { 200 } else { 60 };
            p.copy_from_slice(&[b, g, r, 255]);
        }
    }
    Rc::new(sf)
}

impl Images {
    pub fn page(&mut self, ui: &mut Ui) {
        let (big, picture) = self.made.get_or_insert_with(|| (pattern(VIEW_SIZE, VIEW_SIZE, 0), pattern(PICTURE_W, PICTURE_H, 1)));
        ui.text(TS_BODY_STRONG, t("Bildansicht"));
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Ziehen verschiebt, das Mausrad zoomt: die Kacheln liegen auf der GPU."));
        ui.next_size(SIZE_FILL, 320);
        self.view.label = alloc::string::String::from(t("Muster"));
        ui.image_view(ID_VIEW, &mut self.view, ViewImage::Bitmap(big, 1), 0);
        ui.text(TS_BODY_STRONG, t("Großes Bild"));
        ui.image(0, picture, 1);
    }
}
