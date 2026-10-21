// Per-face raster policy. Layout advances, document fonts and legacy renderers
// keep their original contracts; styles opt in only for their own UI faces.
use crate::Text;

/// Glyph rasterization independent of font metrics and text editing coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontRaster {
    /// Original x-height fitting and downward quarter-pixel positioning.
    #[default]
    Legacy,
    /// Preserve the outline scale and round to the nearest quarter pixel.
    Natural,
}

impl FontRaster {
    pub(crate) fn position(self, pen: i32) -> i32 {
        match self {
            Self::Legacy => pen,
            Self::Natural => ((pen as i64 + if pen < 0 { 31 } else { 32 }) >> 6)
                .saturating_mul(64).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        }
    }
}

impl Text {
    /// Raster policy of a registered face. Unknown ids follow the fallback face.
    pub fn font_raster(&self, font: u32) -> FontRaster {
        self.slot_idx(font).map_or(FontRaster::Legacy, |i| self.slots[i].raster)
    }

    /// Change only this face's rasterization; invalidate cached masks on change.
    /// Returns false for an unregistered id. Measuring and caret advances are unchanged.
    pub fn set_font_raster(&mut self, font: u32, raster: FontRaster) -> bool {
        let Some(i) = self.slot_idx(font).filter(|i| *i == font as usize) else { return false; };
        if self.slots[i].raster != raster {
            self.slots[i].raster = raster;
            self.gc_flush();
        }
        true
    }
}
