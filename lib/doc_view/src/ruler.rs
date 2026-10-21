// Physical page rulers shared by document editors. Geometry uses twips, never text pixels.
// Independently implemented for jfguiLib; the caller supplies document and view state.

use alloc::string::ToString;
use doc_core::{PageSettings, ParagraphFormat};
use gfx::{path::Path, raster::Raster};
use ui::{Draw, Gfx};

/// Space reserved for a document ruler, in device-independent pixels.
pub const RULER_SIZE: i32 = 22;
/// Space around and between pages, in device-independent pixels.
pub const PAGE_GAP: i32 = 18;

/// Presentation colors of a ruler; these never change stored document colors.
#[derive(Clone, Copy)]
pub struct RulerPalette {
    /// The surrounding document workspace.
    pub background: u32,
    /// The ruler's writable body area.
    pub body: u32,
    /// The ruler's margin area.
    pub margin: u32,
    /// Tick marks, labels and indent outlines.
    pub tick: u32,
}
impl RulerPalette {
    /// Neutral document-ruler colors for a light workspace.
    pub fn light(background: u32) -> Self {
        Self {
            background,
            body: 0xffffffff,
            margin: 0xffd4d4d4,
            tick: 0xff707070,
        }
    }
    /// Neutral document-ruler colors for a dark workspace.
    pub fn dark(background: u32) -> Self {
        Self {
            background,
            body: 0xff2e2e2e,
            margin: 0xff454545,
            tick: 0xffa0a0a0,
        }
    }
}

/// Which physical direction a ruler measures.
#[derive(Clone, Copy)]
pub enum RulerAxis {
    /// Inches along the page width, with paragraph indent markers.
    Horizontal,
    /// Inches along the page height, aligned with the page's scroll offset.
    Vertical,
}

/// Retained ruler drawing, aligned to the same displayed page rectangle as `PageDraw`.
pub struct RulerDraw {
    /// A constrained text area such as a table cell: show indents/tabs, lock its margins.
    pub paragraph_only: bool,
    /// The physical page and margins.
    pub page: PageSettings,
    /// The selected paragraph's indents; ignored by the vertical ruler.
    pub paragraph: ParagraphFormat,
    /// The physical direction of the ruler.
    pub axis: RulerAxis,
    /// Page start in pixels, relative to this drawing's origin on the measured axis.
    pub page_start: i32,
    /// Displayed page width in pixels, defining exactly the editor's scale.
    pub page_width: i32,
    /// Device scale (96 is 100%).
    pub dpi: u32,
    /// Registered UI face used for ruler numbers.
    pub font: u32,
    /// View colors, independent of document formatting.
    pub palette: RulerPalette,
}
impl RulerDraw {
    pub(super) fn valid(&self) -> bool {
        if self.paragraph_only {
            self.page.width > 0 && self.page.width <= 31680 && self.page.left.checked_add(self.page.right).is_some_and(|v| v < self.page.width)
        } else { self.page.validate().is_ok() }
    }
    /// Content fingerprint for retained UI drawing. A selection can change indent
    /// markers without changing the document revision or the drawing's allocation.
    pub fn version(&self) -> u32 {
        let values = [u32::from(self.paragraph_only), self.page.width, self.page.height, self.page.left, self.page.right,
            self.page.top, self.page.bottom, u32::from(self.paragraph.left),
            u32::from(self.paragraph.right), self.paragraph.first_line as u32,
            u32::from(matches!(self.axis, RulerAxis::Vertical)), self.page_start as u32,
            self.page_width as u32, self.dpi, self.font, self.palette.background,
            self.palette.body, self.palette.margin, self.palette.tick];
        let hash = |h: u32, value: u32| value.to_le_bytes().into_iter().fold(h, |h, byte| (h ^ u32::from(byte)).wrapping_mul(0x01000193));
        self.paragraph.tabs.as_slice().iter().fold(values.into_iter().fold(0x811c9dc5u32, hash), |h, s| hash(hash(hash(h, s.position as u32), s.alignment as u32), s.leader as u32))
    }
    pub(super) fn scale(&self, twips: i32) -> i32 {
        (i64::from(twips) * i64::from(self.page_width) / i64::from(self.page.width.max(1))) as i32
    }
    pub(super) fn dp(&self, dip: i32) -> i32 {
        ((i64::from(dip) * i64::from(self.dpi) + 48) / 96) as i32
    }
    fn triangle(&self, g: &mut Gfx, x: i32, y: i32, direction: i32) {
        let mut path = Path::default();
        let mut raster = Raster::default();
        let half = (self.dp(9) * 256) / 2;
        path.begin(g.dc);
        path.move_fx(x * 256 - half, y * 256);
        path.line_fx(x * 256 + half, y * 256);
        path.line_fx(x * 256, (y + direction * self.dp(5)) * 256);
        path.close();
        g.dc.set_color(self.palette.body);
        path.fill(&mut raster, g.dc, g.sf, 0);
        g.dc.set_color(self.palette.tick);
        path.stroke(&mut raster, g.dc, g.sf, self.dp(1).max(1) * 256, 0);
    }
}
impl Draw for RulerDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        if !self.valid() || !(1..=100_000).contains(&self.page_width) {
            return;
        }
        let horizontal = matches!(self.axis, RulerAxis::Horizontal);
        let (length, before, after) = if horizontal {
            (self.page.width, self.page.left, self.page.right)
        } else {
            (self.page.height, self.page.top, self.page.bottom)
        };
        let start = (if horizontal { r[0] } else { r[1] }) + self.page_start;
        let across = (if horizontal { r[1] } else { r[0] }) + self.dp(4);
        let thickness = (if horizontal { r[3] } else { r[2] }) - self.dp(7);
        let fill = |g: &mut Gfx, pos: i32, len: i32, cross: i32, thick: i32| {
            if horizontal {
                g.dc.fill(g.sf, pos, cross, len, thick);
            } else {
                g.dc.fill(g.sf, cross, pos, thick, len);
            }
        };
        g.dc.clip_push(r[0], r[1], r[2], r[3]);
        g.dc.set_color(self.palette.background);
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        g.dc.set_color(self.palette.margin);
        fill(g, start, self.scale(length as i32), across, thickness);
        g.dc.set_color(self.palette.body);
        fill(
            g,
            start + self.scale(before as i32),
            self.scale(length.saturating_sub(before.saturating_add(after)) as i32),
            across,
            thickness,
        );
        let previous = g.text.get_font();
        g.text
            .set_font(self.font, (u64::from(self.dpi) * 19 * 128 / 96) as i32);
        let middle = across + thickness / 2;
        // One eighth of an inch is 180 twips. Numbers are relative to the text origin.
        let first = -(before as i32 / 180) - 1;
        let last = ((length as i64 - before as i64) / 180).min(2048) as i32;
        for step in first..=last {
            let physical = before as i64 + i64::from(step) * 180;
            if physical < 0 || physical > i64::from(length) {
                continue;
            }
            let at = start + self.scale(physical as i32);
            g.dc.set_color(self.palette.tick);
            if step % 8 == 0 && step != 0 {
                let label = (step / 8).abs().to_string();
                let width = g.text.width(&label);
                let (x, y) = if horizontal {
                    (
                        at - width / 2,
                        middle + (g.text.ascent() - g.text.descent()) / 2,
                    )
                } else {
                    (
                        middle - width / 2,
                        at + (g.text.ascent() - g.text.descent()) / 2,
                    )
                };
                g.text.draw(g.dc, g.sf, x, y, label.as_bytes());
            } else if (horizontal && step % 8 != 0) || (!horizontal && step % 4 == 0) {
                if horizontal && self.scale(180) <= self.dp(4) && step % 2 != 0 {
                    continue;
                }
                let tick = self
                    .dp(if step % 4 == 0 {
                        5
                    } else if step % 2 == 0 {
                        3
                    } else {
                        2
                    })
                    .max(1);
                fill(g, at, self.dp(1).max(1), middle - tick / 2, tick);
            }
        }
        if horizontal {
            for tab in self.paragraph.tabs.as_slice() {
                use doc_core::TabAlignment;
                if tab.alignment == TabAlignment::Clear { continue; }
                let x = start + self.scale(before as i32 + tab.position);
                let bottom = across + thickness - self.dp(1);
                let stroke = self.dp(1).max(1);
                let half = self.dp(4);
                g.dc.set_color(self.palette.tick);
                g.dc.fill(g.sf, x, bottom - self.dp(7), stroke, self.dp(7));
                let (left, width) = match tab.alignment { TabAlignment::Left => (x, half), TabAlignment::Right => (x - half, half), TabAlignment::Bar => (x, stroke), _ => (x - half, half * 2) };
                g.dc.fill(g.sf, left, bottom, width, stroke);
                if tab.alignment == TabAlignment::Decimal { g.dc.fill(g.sf, x + self.dp(3), bottom - self.dp(3), stroke * 2, stroke * 2); }
            }
            let left = start + self.scale(before as i32 + i32::from(self.paragraph.left));
            let first = left + self.scale(self.paragraph.first_line);
            let right =
                start + self.scale(length as i32 - after as i32 - i32::from(self.paragraph.right));
            self.triangle(g, first, across, 1);
            self.triangle(g, left, across + thickness - self.dp(3), -1);
            self.triangle(g, right, across + thickness - self.dp(1), -1);
            let half = self.dp(9) / 2;
            let top = across + thickness - self.dp(3);
            let h = self.dp(4).max(1);
            let w = self.dp(9).max(1);
            g.dc.set_color(self.palette.tick);
            g.dc.fill(g.sf, left - half, top, w, h);
            g.dc.set_color(self.palette.body);
            let stroke = self.dp(1).max(1);
            g.dc.fill(
                g.sf,
                left - half + stroke,
                top + stroke,
                w - stroke * 2,
                h - stroke * 2,
            );
        }
        g.text.set_font(previous.0, previous.1);
        g.dc.clip_pop();
    }
}
