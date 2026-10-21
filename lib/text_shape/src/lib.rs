//! OpenType shaping and Unicode bidi boundary, built with locked Cargo dependencies.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

use alloc::vec::Vec;
use harfrust::{Buffer, Direction, Font, ShaperFont};
use unicode_bidi::BidiInfo;
use unicode_script::{Script, UnicodeScript};

/// A glyph in visual order, with metrics in font units and a UTF-8 input cluster.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub font: usize,
    pub id: u32, pub cluster: usize, pub end: usize, pub advance: i32,
    pub x: i32, pub y: i32, pub rtl: bool,
}
pub struct Engine { font: Font }
impl Engine {
    pub fn new(data: &[u8], face: u32) -> Option<Self> { Some(Self { font: Font::new(data.to_vec(), face)? }) }
    /// One line, split into directional and script runs. Shaping itself handles
    /// ligatures, combining marks, Arabic joining and Indic reordering.
    pub fn shape(&self, text: &str) -> Option<Vec<Glyph>> {
        self.shape_with_fallback(text,&[])
    }
    pub fn shape_with_fallback(&self, text: &str, fallbacks: &[&Engine]) -> Option<Vec<Glyph>> {
        if text.len() > 65536 { return None; }
        let bidi = BidiInfo::new(text,None);
        let mut result = Vec::new();
        for para in &bidi.paragraphs {
            let (levels,runs) = bidi.visual_runs(para,para.range.clone());
            for run in runs {
                let rtl = levels[run.start].is_rtl();
                let mut segments = Vec::new(); let mut start=run.start; let mut script=Script::Common;
                for (offset,ch) in text[run.clone()].char_indices() {
                    let next=ch.script();
                    if matches!(next,Script::Common|Script::Inherited|Script::Unknown) { continue; }
                    if script != Script::Common && script != next { segments.push(start..run.start+offset); start=run.start+offset; }
                    script=next;
                }
                segments.push(start..run.end);
                if rtl { segments.reverse(); }
                for segment in segments {
                    let mut font_index=0;
                    let mut font=ShaperFont::new(&self.font);
                    let covers=|f:&ShaperFont| text[segment.clone()].chars().all(|c| matches!(c as u32,0x200C..=0x200F|0x202A..=0x202E|0x2066..=0x2069|0xFE00..=0xFE0F) || f.nominal_glyph(c as u32).is_some());
                    if !covers(&font) {
                        for (i,engine) in fallbacks.iter().enumerate() {
                            let candidate=ShaperFont::new(&engine.font);
                            if covers(&candidate) { font=candidate; font_index=i+1; break; }
                        }
                    }
                    let mut buffer=Buffer::new(); buffer.push_str(&text[segment.clone()]);
                    buffer.set_direction(if rtl { Direction::RightToLeft } else { Direction::LeftToRight });
                    buffer.guess_segment_properties();
                    harfrust::shape(&font,&mut buffer,Default::default()).ok()?;
                    let mut boundaries: Vec<usize> = buffer.glyph_infos().iter().map(|g| segment.start+g.cluster as usize).collect();
                    boundaries.push(segment.end); boundaries.sort_unstable(); boundaries.dedup();
                    for (g,p) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
                        let cluster=segment.start+g.cluster as usize;
                        let end=*boundaries.get(boundaries.binary_search(&cluster).ok()?+1)?;
                        result.push(Glyph { font:font_index, id:g.glyph_id, cluster, end, advance:p.x_advance, x:p.x_offset, y:p.y_offset, rtl });
                    }
                }
            }
        }
        Some(result)
    }
}
