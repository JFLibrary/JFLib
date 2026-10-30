// Input/selection overlay for externally laid out rich text. New opt-in UI API.
// The caller paints content; the existing textbox engine handles Unicode, clipboard and IME.
use super::*;

/// One visual line of a text editing surface, in absolute physical window coordinates.
#[derive(Clone, Debug)]
pub struct TextSurfaceLine {
    /// UTF-8 byte range, excluding an explicit newline. Soft wraps share their boundary.
    pub start: usize,
    pub end: usize,
    /// Hit/selection rectangle [x, y, width, height]. It may cover a different page per line.
    pub rect: [i32; 4],
    /// UTF-8 byte caret stops and their absolute x coordinates, in logical order.
    pub carets: Vec<(usize, i32)>,
}
impl TextSurfaceLine {
    fn x(&self, at: usize) -> i32 {
        self.carets
            .iter()
            .rev()
            .find(|(offset, _)| *offset <= at)
            .map_or(self.rect[0], |(_, x)| *x)
    }
    fn hit(&self, x: i32) -> usize {
        self.carets
            .iter()
            .min_by_key(|(_, px)| (i64::from(*px) - i64::from(x)).abs())
            .map_or(self.start, |(offset, _)| *offset)
    }
}
impl Ui {
    /// Edit one UTF-8 buffer over caller-rendered rich text, without repainting its content.
    /// Layout may span differently sized/aligned lines and pages; each UTF-8 scalar boundary
    /// needs a caret stop. Text changes report EV_ACT and request a fresh layout on the next
    /// pass. Invalid ranges reject the call without consuming input. The caller owns scroll
    /// positioning and document history. Ordinary textboxes are unaffected.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn textbox_surface(
        &mut self,
        id: u32,
        text: &mut String,
        cap_flags: u32,
        lines: &[TextSurfaceLine],
    ) -> u32 {
        let _src = self.src_scope();
        self.m.edit.surface_change = None;
        self.m.edit.surface_start = None;
        if lines.is_empty()
            || lines.len() > 100000
            || lines.iter().any(|line| {
                line.start > line.end
                    || !text.is_char_boundary(line.start)
                    || !text.is_char_boundary(line.end)
                    || line.rect[2] < 1
                    || line.rect[3] < 1
                    || line.carets.is_empty()
                    || line.carets.iter().any(|(at, _)| {
                        *at < line.start || *at > line.end || !text.is_char_boundary(*at)
                    })
                    || line.carets.first().map(|(at, _)| *at) != Some(line.start)
                    || line.carets.last().map(|(at, _)| *at) != Some(line.end)
                    || line.carets.windows(2).any(|p| p[0].0 >= p[1].0)
                    || line.carets.len() != text[line.start..line.end].chars().count() + 1
            })
            || lines.windows(2).any(|p| p[0].end > p[1].start)
        {
            return 0;
        }
        let original = text.clone();
        let mut bytes = core::mem::take(text).into_bytes();
        core::mem::swap(&mut bytes, &mut self.m.edit.buf);
        let prior = self.m.edit.surface.replace(lines.to_vec());
        let flags = self.tb_start(id, cap_flags | TB_MULTI, TB_FLAG_MASK & !TB_PASSWORD);
        let mut ev = 0;
        let hb = HB_FOCUS
            | HB_TEXT
            | CUR_IBEAM << HB_CURSOR_SHIFT
            | if flags & ST_DISABLED != 0 {
                HB_DISABLED
            } else {
                0
            };
        let mut registered = false;
        for line in lines {
            let r = line.rect;
            let before = self.wp.ctx.hits[self.wp.ctx.hit_cur].len();
            ev |= self.hit_rect(id, r[0], r[1], r[2], r[3], hb);
            if let Some(hit) = self.wp.ctx.hits[self.wp.ctx.hit_cur].get_mut(before) {
                hit.continuation = registered;
                registered = true;
            }
        }
        let mut editing = self.tb_focus();
        if editing {
            self.m.edit.ed_multi = true;
            self.m.edit.ml_page = 20;
            let c = &self.wp.ctx;
            if c.pressed & (1 << BTN_LEFT) != 0 && ev & EV_HOVER != 0 && c.active == id {
                let at = self.surface_hit(self.ltr_x(c.mx), c.my);
                self.m.edit.ed_prefx = -1;
                self.tb_press(ev, at);
            } else if ev & EV_HELD != 0 && self.m.edit.ed_drag {
                let at = self.surface_hit(self.ltr_x(c.mx), c.my);
                self.tb_drag_to(at);
            } else {
                self.m.edit.ed_drag = false;
            }
            self.tb_keys();
            self.tb_typed();
            if self.m.edit.tb_chg {
                self.m.edit.tb_ev |= EV_ACT;
                self.repass();
            }
            editing = self.m.edit.ed_id == id;
        }
        if editing && !self.m.edit.tb_chg {
            let (a, b) = self.tb_sel();
            for line in lines {
                let lo = a.max(line.start);
                let hi = b.min(line.end);
                if lo < hi || (lo == hi && a <= line.end && b > line.end) {
                    let x = line.x(lo);
                    let end = line
                        .x(hi)
                        .saturating_add(if b > line.end { self.dp(4) } else { 0 });
                    self.draw_color((self.color(T_ACCENT) & 0x00ffffff) | 0x50000000);
                    self.draw_rect(x, line.rect[1], (end - x).max(1), line.rect[3]);
                }
            }
            let line = &lines[self.surface_line(self.m.edit.ed_caret)];
            let x = line.x(self.m.edit.ed_caret);
            if self.tb_caret_on() {
                self.draw_color(0xff20272b);
                self.draw_rect(x, line.rect[1], self.hair(), line.rect[3]);
            }
            self.ime_pos(x, line.rect[1], line.rect[3]);
        }
        self.wp.ctx.last_rect = lines[if editing {
            self.surface_line(self.m.edit.ed_caret)
        } else {
            0
        }]
        .rect;
        let result = self.tb_finish(ev, editing);
        self.m.edit.surface = prior;
        core::mem::swap(&mut bytes, &mut self.m.edit.buf);
        *text = to_string(bytes);
        if original != *text {
            // Bound the prefix by the actual edit location: equal adjacent characters must
            // inherit the format at the insertion point, not at a later matching seam.
            let start = self
                .m
                .edit
                .surface_start
                .unwrap_or(0)
                .min(original.len())
                .min(text.len());
            let start = Self::tb_fix_index(original.as_bytes(), start);
            let mut suffix = original.as_bytes()[start..]
                .iter()
                .rev()
                .zip(text.as_bytes()[start..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            while !original.is_char_boundary(original.len() - suffix)
                || !text.is_char_boundary(text.len() - suffix)
            {
                suffix -= 1;
            }
            self.m.edit.surface_change = Some((
                start,
                original.len() - suffix,
                String::from(&text[start..text.len() - suffix]),
            ));
        }
        result
    }
    /// Replacement in the original buffer from the most recent surface call. The range
    /// respects the actual insertion location even when surrounding characters repeat.
    pub fn textbox_surface_change(&self) -> Option<(usize, usize, String)> {
        self.m.edit.surface_change.clone()
    }
    fn surface_line(&self, offset: usize) -> usize {
        self.m
            .edit
            .surface
            .as_ref()
            .unwrap()
            .iter()
            .rposition(|l| l.start <= offset)
            .unwrap_or(0)
    }
    fn surface_hit(&self, x: i32, y: i32) -> usize {
        let lines = self.m.edit.surface.as_ref().unwrap();
        let line = lines
            .iter()
            .min_by_key(|line| {
                let top = i64::from(line.rect[1]);
                let end = top + i64::from(line.rect[3]);
                if i64::from(y) < top {
                    top - i64::from(y)
                } else {
                    (i64::from(y) - end).max(0)
                }
            })
            .unwrap();
        Self::tb_fix_index(&self.m.edit.buf, line.hit(x))
    }
    pub(super) fn surface_vmove(&mut self, delta: i32, shift: bool) {
        let lines = self.m.edit.surface.as_ref().unwrap();
        let current = self.surface_line(self.m.edit.ed_caret);
        let x = if self.m.edit.ed_prefx < 0 {
            lines[current].x(self.m.edit.ed_caret)
        } else {
            self.m.edit.ed_prefx
        };
        let next = current as i64 + i64::from(delta);
        let at = if next < 0 {
            0
        } else if next >= lines.len() as i64 {
            self.m.edit.buf.len()
        } else {
            Self::tb_fix_index(&self.m.edit.buf, lines[next as usize].hit(x))
        };
        self.tb_move(at, shift);
        self.m.edit.ed_prefx = x;
    }
}
