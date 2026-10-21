// Shared document presentation: stable fonts, pagination measurement and page drawing.
// Extracted from jfWord's existing page adapter; no legacy or WordCraft counterpart.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod coords;
mod fonts;
mod measure;
mod page;
mod cell_borders;
mod cell_border_editor;
pub use cell_border_editor::CellBorderEditor;
mod table;
mod table_editor;
mod formula_editor;
pub use formula_editor::FormulaEditor;
mod table_style_editor;
pub use table_style_editor::TableStyleEditor;
pub use table_editor::TableProperties;
mod table_position_editor;
pub use table_position_editor::TablePositionEditor;
pub use table::{TableController,table_cell_at,text_line_bounds};
mod ruler;
mod ruler_input;
mod tab_editor;

pub use coords::{PageTransform, page_height, zoom_width};
pub use fonts::{FontError, Fonts};
pub use measure::{FontMeasure, paginate, fit_table, apply_paginated, replace_fragment_paginated};
pub use page::{PageDraw, PageError, page_backdrop};
pub use ruler::{PAGE_GAP, RULER_SIZE, RulerAxis, RulerDraw, RulerPalette};
pub use ruler_input::{RulerController, RulerEdit, RulerHandle};
pub use tab_editor::{TabEditor, tab_selector, ALIGNMENTS as TAB_ALIGNMENTS};

/// Document viewport gutter reserved for the vertical scrollbar (DIP).
/// Exclude it from page centering; it does not change document dimensions or pagination.
pub const SCROLL_GUTTER: i32 = 14;

/// Compact style-gallery typography, preserving relative script size and displacement.
/// The caller prepares this family in the text engine before requesting a sample.
pub fn ribbon_sample(format: doc_core::ResolvedFormat, fonts: &Fonts) -> ui::RibbonSample {
    let nominal = (i32::from(format.size_half_points) * 512 / 3).clamp(9 * 256, 26 * 256);
    let size = i32::from(format.size_half_points).max(1);
    ui::RibbonSample { font: fonts.get(format), size: nominal * i32::from(format.display_size_half_points()) / size,
        color: format.color, highlight: format.highlight.rgb().map(|rgb| 0xff000000 | rgb),
        baseline_shift: nominal * format.baseline_shift() / (size * 10), underline: format.underline, strike: format.strike }
}
