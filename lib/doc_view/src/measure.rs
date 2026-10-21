// Font measurement adapter between doc_layout's twips and the shared text engine.
// Operation order matches the previous jfWord measurement to preserve pagination.

use doc_core::{Document, ResolvedFormat};
use doc_layout::{Layout, Measure, Metrics};
use text::Text;

use crate::Fonts;

/// Measures document text in twips with a registered document font set.
pub struct FontMeasure<'a> {
    text: &'a mut Text,
    fonts: &'a Fonts,
}

impl<'a> FontMeasure<'a> {
    /// Borrow the shared text engine for layout. `paginate` also restores its current font.
    pub fn new(text: &'a mut Text, fonts: &'a Fonts) -> Self {
        Self { text, fonts }
    }
}

impl Measure for FontMeasure<'_> {
    fn width(&mut self, value: &str, format: ResolvedFormat) -> i32 {
        self.text.set_font(
            self.fonts.get(format),
            format.display_size_half_points() as i32 * 10 * 256,
        );
        (self.text.measure(value) + 128) / 256
    }

    fn metrics(&mut self, format: ResolvedFormat) -> Metrics {
        self.text.set_font(
            self.fonts.get(format),
            format.display_size_half_points() as i32 * 10 * 256,
        );
        let ascent = self.text.ascent();
        let descent = self.text.descent();
        Metrics {
            ascent,
            descent,
            height: self.text.line_h().max(ascent + descent),
        }
    }
}

/// Paginate with document fonts and restore the text engine's previous UI font on return.
pub fn paginate(
    document: &Document,
    text: &mut Text,
    fonts: &Fonts,
) -> Result<Layout, doc_layout::Error> {
    fonts.prepare(text, document);
    if !fonts.valid(text) {
        return Err(doc_layout::Error::Metrics);
    }
    let previous = text.get_font();
    let result = doc_layout::paginate(document, &mut FontMeasure::new(text, fonts));
    text.set_font(previous.0, previous.1);
    result
}

/// Commit only if the new document can be paginated with the actual document fonts.
/// Model errors and layout errors preserve the editor's revision and undo history.
pub fn apply_paginated(editor:&mut doc_core::Editor,edit:doc_core::Edit,text:&mut Text,fonts:&Fonts)->Result<doc_core::Id,doc_layout::Error>{
    let mut layout_error=None;
    let result=editor.apply_checked(edit,|d|match paginate(d,text,fonts){Ok(_)=>Ok(()),Err(e)=>{layout_error=Some(e);Err(doc_core::Error::Limit)}});
    result.map_err(|e|layout_error.unwrap_or(doc_layout::Error::Model(e)))
}

/// Paste a fragment only if it fits the page layout; return its surviving trailing caret.
pub fn replace_fragment_paginated(editor:&mut doc_core::Editor,paragraph:doc_core::Id,start:usize,end:usize,fragment:doc_core::DocumentFragment,text:&mut Text,fonts:&Fonts)->Result<(doc_core::Id,usize),doc_layout::Error>{
    let mut layout_error=None;
    let result=editor.replace_story_fragment_checked(paragraph,start,end,fragment,|d|match paginate(d,text,fonts){Ok(_)=>Ok(()),Err(e)=>{layout_error=Some(e);Err(doc_core::Error::Limit)}});
    result.map_err(|e|layout_error.unwrap_or(doc_layout::Error::Model(e)))
}

/// Compute table widths while preserving the UI's current font and font size.
pub fn fit_table(document:&Document,id:doc_core::Id,mode:doc_layout::TableFit,text:&mut Text,fonts:&Fonts)->Result<alloc::vec::Vec<u32>,doc_layout::Error> {
    fonts.prepare(text,document);
    if !fonts.valid(text) {return Err(doc_layout::Error::Metrics);}
    let previous=text.get_font();
    let result=doc_layout::table_fit(document,id,mode,&mut FontMeasure::new(text,fonts));
    text.set_font(previous.0,previous.1);
    result
}
