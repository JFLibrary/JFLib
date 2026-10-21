// jfWord preview navigation; reusable page fonts, measurement and drawing live in doc_view.
use alloc::{format, rc::Rc, string::String, vec::Vec};
use doc_core::Document;
use doc_layout::Layout;
pub(crate) use doc_view::Fonts;
use doc_view::{PageDraw, PageTransform};
use ui::*;

pub(crate) struct Preview {
    revision: Option<u64>,
    fonts: Option<Fonts>,
    layout: Option<Rc<Layout>>,
    draws: Vec<DrawRef>,
    error: String,
    zoom: i32,
    fit_width: bool,
    page: i32,
}
impl Default for Preview {
    fn default() -> Self {
        Self {
            revision: None,
            fonts: None,
            layout: None,
            draws: Vec::new(),
            error: String::new(),
            zoom: 100,
            fit_width: false,
            page: 0,
        }
    }
}
impl Preview {
    pub(crate) fn fonts(&self) -> Option<Fonts> {
        self.fonts.clone()
    }
    pub(crate) fn reset(&mut self) {
        let fonts = self.fonts.clone();
        *self = Self::default();
        self.fonts = fonts;
    }
    pub(crate) fn prepare_fonts(
        &mut self,
        text: &mut text::Text,
    ) -> Result<Fonts, doc_view::FontError> {
        if let Some(fonts) = self.fonts.clone() {
            return Ok(fonts);
        }
        let fonts = Fonts::new(text)?;
        self.fonts = Some(fonts.clone());
        Ok(fonts)
    }
    pub(crate) fn update(&mut self, ui: &mut Ui, d: &Document, revision: u64) {
        if self.revision != Some(revision) {
            let fonts = match self.prepare_fonts(&mut ui.text) {
                Ok(fonts) => fonts,
                Err(error) => {
                    self.layout = None;
                    self.draws.clear();
                    self.error = format!("Dokumentschrift nicht verfügbar: {error:?}");
                    return;
                }
            };
            let result = doc_view::paginate(d, &mut ui.text, &fonts);
            self.draws.clear();
            match result {
                Ok(layout) => {
                    let layout = Rc::new(layout);
                    for index in 0..layout.pages.len() {
                        match PageDraw::new(layout.clone(), index, fonts.clone()) {
                            Ok(draw) => self.draws.push(Rc::new(draw)),
                            Err(error) => {
                                self.layout = None;
                                self.draws.clear();
                                self.error = format!("Seite nicht darstellbar: {error:?}");
                                self.revision = Some(revision);
                                return;
                            }
                        }
                    }
                    self.layout = Some(layout);
                    self.error.clear();
                }
                Err(e) => {
                    self.layout = None;
                    self.error = format!("Seitenvorschau nicht verfügbar: {e:?}");
                }
            }
            self.revision = Some(revision);
        }
    }
    pub(crate) fn layout(&self) -> Option<Rc<Layout>> {
        self.layout.clone()
    }
    pub(crate) fn draw(&self, index: usize) -> DrawRef {
        self.draws[index].clone()
    }
    pub(crate) fn error(&self) -> &str {
        &self.error
    }
    pub(crate) fn zoom_control(&mut self, ui: &mut Ui) {
        ui.next_size(22, 20);
        ui.next_flags(ST_SUBTLE | ST_COMPACT);
        if ui.icon_button(2800, IC_ZOOM_OUT, "Verkleinern") & EV_ACT != 0 {
            self.set_zoom(self.zoom - 10);
        }
        ui.next_size(110, 20);
        let mut value = self.zoom;
        if ui.slider(2801, &mut value, &Range::new(10, 500, 1, RG_TIP | RG_LOG)) & EV_ACT != 0 {
            self.set_zoom(value);
        }
        ui.next_size(22, 20);
        ui.next_flags(ST_SUBTLE | ST_COMPACT);
        if ui.icon_button(2802, IC_ZOOM_IN, "Vergrößern") & EV_ACT != 0 {
            self.set_zoom(self.zoom + 10);
        }
        // Preserve the original six choices and widget ID for automation compatibility.
        const VALUES: [i32; 12] = [50, 75, 100, 125, 150, 0, 10, 25, 200, 300, 400, 500];
        let mut choice = if self.fit_width {
            5
        } else {
            VALUES.iter().position(|v| *v == self.zoom).unwrap_or(12) as i32
        };
        let custom = format!("{} %", self.zoom);
        let labels = [
            "50 %",
            "75 %",
            "100 %",
            "125 %",
            "150 %",
            "Seitenbreite",
            "10 %",
            "25 %",
            "200 %",
            "300 %",
            "400 %",
            "500 %",
            &custom,
        ];
        ui.next_size(if self.fit_width { 80 } else { 48 }, 20);
        ui.next_flags(ST_COMPACT | ST_SUBTLE);
        let previous = choice;
        ui.combobox(
            126,
            &mut choice,
            &labels[..if previous == 12 { 13 } else { 12 }],
        );
        if choice != previous {
            if choice == 5 {
                self.set_fit_width();
            } else if let Some(value) = VALUES.get(choice as usize) {
                self.set_zoom(*value);
            }
        }
    }
    pub(crate) fn zoom_percent(&self) -> i32 {
        self.zoom
    }
    pub(crate) fn is_fit_width(&self) -> bool {
        self.fit_width
    }
    pub(crate) fn set_zoom(&mut self, percent: i32) {
        self.zoom = percent.clamp(10, 500);
        self.fit_width = false;
    }
    pub(crate) fn set_fit_width(&mut self) {
        self.fit_width = true;
    }
    pub(crate) fn set_page(&mut self, index: usize) {
        self.page = index.min(i32::MAX as usize) as i32;
    }
    pub(crate) fn page_index(&self) -> usize {
        self.page.max(0) as usize
    }
    pub(crate) fn page_width(&mut self, width_twips: u32, available_dip: i32) -> i32 {
        if self.fit_width {
            let width = available_dip.max(1).clamp(
                doc_view::zoom_width(width_twips, 10),
                doc_view::zoom_width(width_twips, 500),
            );
            if width_twips != 0 {
                self.zoom = (width as i64 * 1500 / width_twips as i64).clamp(10, 500) as i32;
            }
            width
        } else {
            doc_view::zoom_width(width_twips, self.zoom)
        }
    }
    pub(crate) fn show(&mut self, ui: &mut Ui, d: &Document, revision: u64) -> Option<u64> {
        self.update(ui, d, revision);
        ui.next_size(-1, -1);
        ui.box_begin(125, BOX_V, 8);
        let mut clicked = None;
        if let Some(layout) = self.layout.clone() {
            ui.box_begin(0, BOX_H, 8);
            ui.text(TS_BODY_STRONG, &format!("{} Seiten", layout.pages.len()));
            if self.page <= 0 {
                ui.next_flags(ST_DISABLED);
            }
            if ui.button(127, "Zurück") & EV_ACT != 0 {
                self.page -= 1;
            }
            if self.page as usize + 1 >= layout.pages.len() {
                ui.next_flags(ST_DISABLED);
            }
            if ui.button(128, "Weiter") & EV_ACT != 0 {
                self.page += 1;
            }
            self.page = self.page.clamp(0, layout.pages.len() as i32 - 1);
            ui.box_end();
            ui.text(
                TS_CAPTION | TX_SECONDARY,
                "Absatz anklicken zum Bearbeiten · jfWord-Seitenumbruch",
            );
            ui.next_size(-1, -1);
            ui.scroll_begin(129, SC_V | SC_H | BOX_V | pad(12));
            let available = ui.px2dip(ui.last_rect()[2]).saturating_sub(24).max(120);
            let w = self.page_width(layout.settings.width, available);
            let h = doc_view::page_height(layout.settings.width, layout.settings.height, w);
            let index = self.page as usize;
            ui.text(
                TS_CAPTION,
                &format!("Vorschauseite {} von {}", index + 1, layout.pages.len()),
            );
            ui.next_size(w, h);
            let input = ui.canvas(130, Some(self.draws[index].clone()), 0, revision as u32);
            if input.ev & EV_ACT != 0 && input.rect[2] > 0 {
                if let Some(transform) = PageTransform::new(layout.settings.width, input.rect) {
                    clicked = layout.paragraph_at(index, transform.unscale(input.y));
                }
            }
            ui.scroll_end();
        } else {
            ui.text(TS_BODY | TX_WRAP, &self.error);
        }
        ui.box_end();
        clicked
    }
}
