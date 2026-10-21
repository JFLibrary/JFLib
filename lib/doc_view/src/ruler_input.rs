//! Shared, reversible ruler gestures. The host applies the returned edit once on release.
use super::{RulerAxis, RulerDraw};
use doc_core::{PageSettings, ParagraphFormat, TabAlignment, TabLeader, TabStop};
use ui::{UiInput, BTN_LEFT, MOD_ALT, VK_DELETE, VK_ESCAPE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulerHandle { FirstLine, Hanging, LeftIndent, RightIndent, MarginStart, MarginEnd, Tab(i32), NewTab }
#[derive(Clone, Copy, Debug)]
pub enum RulerEdit { Paragraph(ParagraphFormat), Page(PageSettings) }
#[derive(Clone, Copy)]
struct Drag { handle: RulerHandle, page: PageSettings, paragraph: ParagraphFormat, context: u64, grab: i32, start: i32, value: i32, removed: bool }
/// One controller per ruler, reusable by any document application on every platform.
#[derive(Default)]
pub struct RulerController { drag: Option<Drag>, selected_tab: Option<i32> }
impl RulerDraw {
    pub fn pixel_at(&self, twips: i32) -> i32 { self.page_start + self.scale(twips) }
    pub fn twips_at(&self, pixel: i32) -> i32 {
        (((i64::from(pixel) - i64::from(self.page_start)) * i64::from(self.page.width) + i64::from(self.page_width.max(1)) / 2) / i64::from(self.page_width.max(1))).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }
    /// Hit geometry shared with drawing, including DPI and the displayed page zoom.
    pub fn handle_at(&self, x: i32, y: i32) -> Option<RulerHandle> {
        let horizontal = matches!(self.axis, RulerAxis::Horizontal);
        let at = if horizontal { x } else { y };
        let close = |twips| (at - self.pixel_at(twips)).abs() <= self.dp(5).max(3);
        if horizontal {
            let left = self.page.left as i32 + i32::from(self.paragraph.left);
            if y >= self.dp(3) && y <= self.dp(10) && close(left + self.paragraph.first_line) { return Some(RulerHandle::FirstLine); }
            if y >= self.dp(10) && close(left) { return Some(if y >= self.dp(16) { RulerHandle::LeftIndent } else { RulerHandle::Hanging }); }
            if y >= self.dp(10) && close((self.page.width - self.page.right) as i32 - i32::from(self.paragraph.right)) { return Some(RulerHandle::RightIndent); }
            if y >= self.dp(8) {
                if let Some(t) = self.paragraph.tabs.as_slice().iter().filter(|t| t.alignment != TabAlignment::Clear).min_by_key(|t| (at - self.pixel_at(self.page.left as i32 + t.position)).abs()) {
                    if close(self.page.left as i32 + t.position) { return Some(RulerHandle::Tab(t.position)); }
                }
            }
        }
        let (start, end) = if horizontal { (self.page.left, self.page.width - self.page.right) } else { (self.page.top, self.page.height - self.page.bottom) };
        if close(start as i32) { if self.paragraph_only { None } else { Some(RulerHandle::MarginStart) } }
        else if close(end as i32) { if self.paragraph_only { None } else { Some(RulerHandle::MarginEnd) } }
        else if horizontal && at > self.pixel_at(start as i32) && at < self.pixel_at(end as i32) { Some(RulerHandle::NewTab) }
        else { None }
    }
}
impl RulerController {
    pub fn cancel(&mut self) { self.drag = None; self.selected_tab = None; }
    pub fn dragging(&self) -> bool { self.drag.is_some() }
    /// Ruler-only preview; the document and its undo stack are untouched during a drag.
    pub fn preview(&self, draw: &mut RulerDraw, alignment: TabAlignment) {
        if let Some(d) = self.drag { if let Some(edit) = edit(d, draw.axis, alignment) {
            match edit {
                RulerEdit::Page(p) => draw.page = p,
                RulerEdit::Paragraph(p) => { draw.paragraph = p.over(d.paragraph); }
            }
        } }
    }
    /// Alt bypasses the 1/16-inch snap. Escape cancels. Dragging a tab away removes it.
    /// `context` must change when the document revision or target selection changes.
    pub fn update(&mut self, draw: &RulerDraw, input: &UiInput, context: u64, alignment: TabAlignment) -> Option<RulerEdit> {
        if !draw.valid() || !(1..=100_000).contains(&draw.page_width) { self.cancel(); return None; }
        if self.drag.is_some_and(|d| d.context != context) { self.cancel(); }
        if input.key & 0xffff == VK_ESCAPE { self.drag = None; return None; }
        if input.key & 0xffff == VK_DELETE && self.drag.is_none() {
            if let Some(pos) = self.selected_tab.take() {
                let mut f = ParagraphFormat::default(); f.tabs.set(TabStop::clear(pos)).ok()?;
                return Some(RulerEdit::Paragraph(f));
            }
        }
        let horizontal = matches!(draw.axis, RulerAxis::Horizontal);
        let axis = if horizontal { input.x } else { input.y };
        let cross = if horizontal { input.y } else { input.x };
        let physical = draw.twips_at(axis);
        if input.pressed & (1 << BTN_LEFT) != 0 {
            let handle = draw.handle_at(input.x, input.y)?;
            let initial = match handle {
                RulerHandle::FirstLine => draw.page.left as i32 + i32::from(draw.paragraph.left) + draw.paragraph.first_line,
                RulerHandle::LeftIndent | RulerHandle::Hanging => draw.page.left as i32 + i32::from(draw.paragraph.left),
                RulerHandle::RightIndent => (draw.page.width - draw.page.right) as i32 - i32::from(draw.paragraph.right),
                RulerHandle::Tab(p) => draw.page.left as i32 + p,
                RulerHandle::MarginStart => (if horizontal { draw.page.left } else { draw.page.top }) as i32,
                RulerHandle::MarginEnd => (if horizontal { draw.page.width - draw.page.right } else { draw.page.height - draw.page.bottom }) as i32,
                RulerHandle::NewTab => physical,
            };
            self.selected_tab = if let RulerHandle::Tab(p) = handle { Some(p) } else { None };
            self.drag = Some(Drag { handle, page: draw.page, paragraph: draw.paragraph, context, grab: physical - initial, start: initial, value: initial, removed: false });
        }
        if let Some(mut d) = self.drag {
            let raw = physical - d.grab;
            // Tab/indent snapping uses the text origin; margins use the paper origin.
            let origin = if horizontal && !matches!(d.handle, RulerHandle::MarginStart | RulerHandle::MarginEnd) { d.page.left as i32 } else { 0 };
            let value = if input.mods & MOD_ALT != 0 { raw } else { origin + ((raw - origin + 45).div_euclid(90)) * 90 };
            let mut candidate = d;
            candidate.value = if raw == d.start && d.handle != RulerHandle::NewTab { d.start } else { value };
            candidate.removed = matches!(d.handle, RulerHandle::Tab(_) | RulerHandle::NewTab) && (cross < -draw.dp(10) || cross > draw.dp(super::RULER_SIZE + 10));
            if edit(candidate, draw.axis, alignment).is_some() || (candidate.removed && d.handle == RulerHandle::NewTab) { d = candidate; }
            self.drag = Some(d);
            if input.released & (1 << BTN_LEFT) != 0 {
                self.drag = None;
                if matches!(d.handle, RulerHandle::Tab(_) | RulerHandle::NewTab) && !d.removed { self.selected_tab = Some(d.value - d.page.left as i32); }
                if d.value != d.start || d.removed || d.handle == RulerHandle::NewTab { return edit(d, draw.axis, alignment); }
            }
        }
        None
    }
}
fn edit(d: Drag, axis: RulerAxis, alignment: TabAlignment) -> Option<RulerEdit> {
    let mut f = ParagraphFormat::default();
    let pos = d.value - d.page.left as i32;
    match d.handle {
        RulerHandle::FirstLine => { f.first_line = pos - i32::from(d.paragraph.left); f.explicit = 64; }
        RulerHandle::LeftIndent | RulerHandle::Hanging => {
            f.left = u16::try_from(pos).ok()?; f.explicit = 16;
            if d.handle == RulerHandle::Hanging { f.first_line = i32::from(d.paragraph.left) + d.paragraph.first_line - pos; f.explicit |= 64; }
        }
        RulerHandle::RightIndent => { f.right = u16::try_from((d.page.width - d.page.right) as i32 - d.value).ok()?; f.explicit = 32; }
        RulerHandle::MarginStart | RulerHandle::MarginEnd => {
            let mut p = d.page;
            let value = u32::try_from(d.value).ok()?;
            match (axis, d.handle) {
                (RulerAxis::Horizontal, RulerHandle::MarginStart) => p.left = value,
                (RulerAxis::Horizontal, _) => p.right = p.width.checked_sub(value)?,
                (RulerAxis::Vertical, RulerHandle::MarginStart) => p.top = value,
                (RulerAxis::Vertical, _) => p.bottom = p.height.checked_sub(value)?,
            }
            p.validate().ok()?; d.paragraph.validate(p).ok()?;
            return Some(RulerEdit::Page(p));
        }
        RulerHandle::Tab(old) => {
            let mut stop = *d.paragraph.tabs.as_slice().iter().find(|s| s.position == old)?;
            if d.removed || pos != old { f.tabs.set(TabStop::clear(old)).ok()?; }
            if !d.removed { stop.position = pos; f.tabs.set(stop).ok()?; }
        }
        RulerHandle::NewTab => {
            if d.removed { return None; }
            f.tabs.set(TabStop { position: pos, alignment, leader: TabLeader::None }).ok()?;
        }
    }
    f.over(d.paragraph).validate(d.page).ok()?;
    Some(RulerEdit::Paragraph(f))
}
