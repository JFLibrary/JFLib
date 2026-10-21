//! Opt-in complex text layout. UI controls keep their existing metrics unless enabled.
use alloc::{vec::Vec, string::String};
use crate::{Text, registry::Face};

#[derive(Default)]
pub(crate) struct Cache {
    pub enabled: bool, serial: u64, font: u32, size: i32,
    engine: Option<text_shape::Engine>, text: String, glyphs: Vec<Placed>, carets: Vec<(usize,i32)>, width: i32,
    fallbacks: Vec<(u32, text_shape::Engine)>,
    segments: Vec<(usize, usize, i32, i32)>,
}
#[derive(Clone, Copy)]
pub(crate) struct Placed { pub font: u32, pub id: u32, pub x: i32, pub y: i32 }
impl Text {
    /// Enable OpenType shaping for measure, draw, hit testing and carets. Returns
    /// the previous state so callers can scope this to a document's text tool.
    pub fn set_shaping(&mut self, enabled: bool) -> bool { core::mem::replace(&mut self.shaping.enabled,enabled) }
    pub fn set_fallback_fonts(&mut self, fonts: &[u32]) {
        if self.fb.app != fonts { self.fallback_app(fonts); }
        let ids:Vec<_>=self.shaping.fallbacks.iter().map(|v|v.0).collect();
        if ids==fonts { return; }
        self.shaping.fallbacks=fonts.iter().take(16).filter_map(|&id| self.shape_face(id).and_then(|f|text_shape::Engine::new(&f.data,f.face_index)).map(|e|(id,e))).collect();
        self.shaping.size=0;
    }
    fn shape_face(&self, mut font: u32) -> Option<&crate::ttf::TtFont> {
        for _ in 0..16 { match self.face(font)? { Face::Tt(tt) => return Some(tt), Face::Syn(s) => font=s.base, _ => return None } }
        None
    }
    pub(crate) fn shape_line(&mut self, bytes: &[u8], limit: Option<usize>) -> bool {
        if !self.shaping.enabled { return false; }
        let n=limit.unwrap_or(bytes.len()).min(bytes.len());
        let n=bytes[..n].iter().position(|b| *b==0 || *b==b'\n').unwrap_or(n);
        let Ok(text)=core::str::from_utf8(&bytes[..n]) else { return false; };
        let render_font=if self.shape_face(self.font).is_some() { self.font } else { match self.shaping.fallbacks.first() { Some(v)=>v.0,None=>return false } };
        let Some(face)=self.shape_face(render_font) else { return false; };
        if self.shaping.serial!=face.serial || self.shaping.engine.is_none() {
            let serial=face.serial; let engine=text_shape::Engine::new(&face.data,face.face_index);
            self.shaping.engine=engine; self.shaping.serial=serial; self.shaping.text.clear(); self.shaping.size=0;
        }
        if self.shaping.engine.is_none() { return false; }
        if self.shaping.font==self.font && self.shaping.size==self.size && self.shaping.text==text { return true; }
        if self.system_fallback() { self.shape_system_fallbacks(text, render_font); }
        let fallback_engines:Vec<_>=self.shaping.fallbacks.iter().map(|v|&v.1).collect();
        let Some(glyphs)=self.shaping.engine.as_ref().and_then(|e| e.shape_with_fallback(text,&fallback_engines)) else { return false; };
        let classic=self.classic_metrics(self.font);
        let scale=|v:i32,font:u32| { let n=(v as i64*self.size as i64/self.upem(font).max(1) as i64).clamp(i32::MIN as i64,i32::MAX as i64) as i32; if classic { n.saturating_add(128)&!255 } else { n } };
        let mut pen=0i32; let mut placed=Vec::new(); let mut carets=Vec::new(); let mut segments=Vec::new();
        let mut i=0;
        while i<glyphs.len() {
            let first=glyphs[i]; let left=pen;
            while i<glyphs.len() && glyphs[i].cluster==first.cluster {
                let g=glyphs[i];
                let font=if g.font==0 { render_font } else { self.shaping.fallbacks[g.font-1].0 };
                placed.push(Placed { font, id:g.id, x:pen.saturating_add(scale(g.x,font)), y:scale(g.y,font) });
                pen=pen.saturating_add(scale(g.advance,font)); i+=1;
            }
            let right=pen; let (begin,end)=if first.rtl { (right,left) } else { (left,right) };
            carets.push((first.cluster,begin));
            // Expose legal UTF-8 caret boundaries even for multi-character ligatures.
            let chars:Vec<_>=text[first.cluster..first.end].char_indices().map(|(o,_)| first.cluster+o).collect();
            for (k,&index) in chars.iter().enumerate() {
                let a=begin+((end-begin) as i64*k as i64/chars.len() as i64) as i32;
                let b=begin+((end-begin) as i64*(k+1) as i64/chars.len() as i64) as i32;
                segments.push((index,chars.get(k+1).copied().unwrap_or(first.end),a.min(b),a.max(b)));
            }
            for (k,&index) in chars.iter().enumerate().skip(1) {
                carets.push((index,begin+((end-begin) as i64*k as i64/chars.len() as i64) as i32));
            }
            carets.push((first.end,end));
        }
        if carets.is_empty() { carets.push((0,0)); }
        carets.sort_by_key(|c| c.0); carets.dedup_by_key(|c| c.0);
        self.shaping.text=String::from(text); self.shaping.font=self.font; self.shaping.size=self.size;
        self.shaping.glyphs=placed; self.shaping.carets=carets; self.shaping.width=pen; self.shaping.segments=segments;
        true
    }
    // system fonts for the characters neither the font nor the fallback engines have
    fn shape_system_fallbacks(&mut self, text: &str, render_font: u32) {
        let mut need: Vec<u32> = Vec::new();
        for ch in text.chars() {
            let cp = ch as u32;
            if self.has(render_font, cp) || self.shaping.fallbacks.iter().any(|f| self.has(f.0, cp)) { continue; }
            if let Some(id) = self.fallback_for(cp) {
                if !need.contains(&id) && !self.shaping.fallbacks.iter().any(|f| f.0 == id) { need.push(id); }
            }
        }
        for id in need {
            if self.shaping.fallbacks.len() >= 16 { break; }
            let engine = self.shape_face(id).and_then(|f| text_shape::Engine::new(&f.data, f.face_index));
            if let Some(e) = engine { self.shaping.fallbacks.push((id, e)); self.shaping.size = 0; }
        }
    }
    pub(crate) fn shaped_draw(&mut self, pen0: i32, mut target: Option<(&mut gfx::Dc,&mut gfx::Surface)>) -> i32 {
        let baseline=self.run_y;
        for i in 0..self.shaping.glyphs.len() {
            let g=self.shaping.glyphs[i];
            if let Some((dc,sf))=&mut target {
                self.run_y=baseline-((g.y+128)>>8);
                if !self.color_glyph(dc,sf,g.font,g.id,pen0.saturating_add(g.x)) { self.glyph_draw(dc,sf,g.font,g.id,pen0.saturating_add(g.x),true); }
            }
        }
        self.run_y=baseline; self.shaping.width
    }
    pub(crate) fn shaped_caret(&self, index: usize) -> i32 {
        self.shaping.carets.iter().min_by_key(|c| c.0.abs_diff(index)).map_or(0,|c| (c.1+128)>>8)
    }
    pub(crate) fn shaped_hit(&self, x: i32) -> usize {
        self.shaping.carets.iter().min_by_key(|c| ((c.1 as i64)-(x as i64*256)).abs()).map_or(0,|c| c.0)
    }
    /// Visual spans for a logical UTF-8 selection. Mixed bidi text may require
    /// disjoint rectangles; the two endpoint carets alone are not sufficient.
    pub fn selection_spans(&mut self, text: &[u8], start: usize, end: usize) -> Vec<(i32,i32)> {
        if start>=end { return Vec::new(); }
        if !self.shape_line(text,None) {
            let a=self.caret_x(text,start); let b=self.caret_x(text,end);
            return alloc::vec![(a.min(b),a.max(b))];
        }
        let mut spans:Vec<_>=self.shaping.segments.iter().filter(|s|s.0<end && s.1>start)
            .map(|s|((s.2+128)>>8,(s.3+128)>>8)).collect();
        spans.sort_unstable();
        let mut merged:Vec<(i32,i32)>=Vec::new();
        for (a,b) in spans {
            if let Some(last)=merged.last_mut() { if a<=last.1 { last.1=last.1.max(b); continue; } }
            merged.push((a,b));
        }
        merged
    }
}
