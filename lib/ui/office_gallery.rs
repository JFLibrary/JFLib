// Reusable document-theme thumbnails drawn with the shared UI rasterizer.
// This is a visual chooser, independent of an application's document commands or formats.
use crate::{Sp, Ui, consts::*};

impl Ui {
    /// Focusable document/template preview card, naturally 150×194 DIP. The supplied
    /// drawing is inset into a white page; its document colors never follow the UI theme.
    /// Hover and selection use the style accent. The full label is exposed to agents.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn preview_card(&mut self, id: u32, label: &str, draw: Option<crate::DrawRef>, user: usize, version: u32) -> u32 {
        let _src = self.src_scope();
        let flags = self.take_flags();
        let r = self.place(self.dp(150), self.dp(194));
        let ev = self.behave(id, HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT
            | if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 });
        let active = flags & ST_DISABLED == 0 && (ev & EV_HOVER != 0 || flags & ST_CHECKED != 0);
        let stroke = self.color(if active { T_ACCENT } else { T_STRONG_STROKE });
        let inset = self.dp(1);
        let offset = self.style.preview_card_offset();
        let inner;
        let clip;
        if let Some([dx, dy]) = offset {
            let (dx, dy) = (self.dp_fx(dx), self.dp_fx(dy));
            let (x, y, w, h) = ((r[0] << 8) + dx, (r[1] << 8) + dy, r[2] << 8, r[3] << 8);
            let border = self.dp_fx(if active { 512 } else { 256 });
            // Two-DIP centerline radius plus half the normal one-DIP border.
            self.cmd_panel_fx(x, y, w, h, self.dp_fx(2 * 256 + 128), border, 0xffffffff, stroke);
            // Round the clip inward: thumbnail fills must not overwrite fractional
            // border pixels. Hover changes the clip, never the preview's scale.
            let inside = |t: i32| {
                let (left, top) = ((x + t + 255) >> 8, (y + t + 255) >> 8);
                [left, top, (((x + w - t) >> 8) - left).max(0), (((y + h - t) >> 8) - top).max(0)]
            };
            inner = inside(self.dp_fx(256));
            clip = inside(border);
        } else {
            self.cmd_rrect(r[0], r[1], r[2], r[3], self.dp(2), self.dp(if active { 2 } else { 1 }), 0xffffffff, stroke, stroke);
            inner = [r[0] + inset, r[1] + inset, (r[2] - 2 * inset).max(0), (r[3] - 2 * inset).max(0)];
            clip = inner;
        }
        if clip[2] > 0 && clip[3] > 0 {
            self.draw_clip(clip[0], clip[1], clip[2], clip[3]);
            self.cmd_custom(inner[0], inner[1], inner[2], inner[3], draw, user, version);
            self.draw_unclip();
        }
        self.focus_ring_draw(ev);
        if flags & ST_CHECKED != 0 { self.wp.ctx.last_state |= SS_CHECKED as u32; }
        self.sem(K_BUTTON, id, Some(label), 0);
        ev
    }

    /// A 56×66-DIP theme chooser containing a 52×50-DIP miniature page.
    /// `theme_color` is a document color, independent of the interface theme.
    /// `variant` selects one of seven sample arrangements (0..=6); larger values wrap.
    /// `next_flags(ST_DISABLED | ST_CHECKED)` controls behavior and selection, while
    /// the document sample remains fully readable. The caller applies its own theme on EV_ACT.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_theme_tile(
        &mut self,
        id: u32,
        label: &str,
        theme_color: u32,
        variant: u32,
    ) -> u32 {
        let _src = self.src_scope();
        let flags = self.take_flags() | ST_SUBTLE;
        let selected = flags & ST_CHECKED != 0;
        let rect = self.place(self.dp(56), self.dp(66));
        let ev = self.behave(
            id,
            HB_FOCUS
                | if flags & ST_DISABLED != 0 {
                    HB_DISABLED
                } else {
                    0
                },
        );
        let mut sp = Sp {
            flags,
            ..Sp::default()
        };
        self.sp_placed(&mut sp, ev);
        let style = self.style;
        style.button(self, &sp);
        let (font, size) = self.text.get_font();
        let x = rect[0] + self.dp(2);
        let y = rect[1] + self.dp(3);
        let w = self.dp(52).min((rect[2] - self.dp(4)).max(0));
        let h = self.dp(50).min((rect[3] - self.dp(6)).max(0));
        if w > 0 && h > 0 {
            let border = self.color(if selected { T_ACCENT } else { T_DIVIDER });
            self.cmd_rrect(x, y, w, h, 0, self.dp(1), 0xffffffff, border, border);
            self.draw_clip(
                x + self.dp(2),
                y + self.dp(2),
                (w - self.dp(4)).max(0),
                (h - self.dp(4)).max(0),
            );
            let variant = variant % 7;
            let ink = theme_color | 0xff000000;
            let inset = self.dp(4);
            let title = if variant == 6 { "TITLE" } else { "Title" };
            self.text.set_font(
                if variant == 1 { FONT_BOLD } else { FONT_REG },
                self.dp_fx(9 * 256),
            );
            let tw = self.text.width(title);
            let tx = if variant == 5 {
                x + (w - tw) / 2
            } else {
                x + inset
            };
            let title_y = y + self.dp(4) + self.text.ascent();
            self.emit_text(tx, title_y, title, 0xff242424);
            if variant == 2 {
                self.draw_color(ink);
                self.draw_rect(x + inset, y + self.dp(17), w - 2 * inset, self.dp(1));
            }
            if variant == 3 {
                self.draw_color(ink);
                self.draw_rect(x + self.dp(1), y + self.dp(26), w - self.dp(2), self.dp(12));
            }
            self.text.set_font(
                if variant == 1 { FONT_BOLD } else { FONT_REG },
                self.dp_fx(6 * 256 + 128),
            );
            let heading = "Heading 1";
            let hw = self.text.width(heading);
            let hx = if variant == 5 {
                x + (w - hw) / 2
            } else {
                x + inset
            };
            self.emit_text(
                hx,
                y + self.dp(28) + self.text.ascent(),
                heading,
                if variant == 3 { 0xffffffff } else { ink },
            );
            if variant == 4 {
                self.draw_color(ink);
                self.draw_rect(x + inset, y + self.dp(37), w - 2 * inset, self.dp(1));
            }
            self.text.set_font(FONT_REG, self.dp_fx(4 * 256 + 128));
            self.emit_text(
                x + inset,
                y + self.dp(40) + self.text.ascent(),
                "Body text in a short",
                0xff242424,
            );
            self.draw_color(0xff707070);
            self.draw_rect(
                x + inset,
                y + self.dp(47),
                (w - 3 * inset).max(0),
                self.dp(1),
            );
            self.draw_unclip();
        }
        self.text.set_font(font, size);
        self.focus_ring_draw(ev);
        if selected {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_BUTTON, id, Some(label), variant as i32);
        self.tooltip(label);
        ev
    }

    /// (JFDiagrammer JD11) a gallery tile with a picture the app made: the theme chooser's
    /// 56 x 66 DIP button (or next_size) with `sf` inside a page frame of 52 x 50 DIP, scaled
    /// to fit and centred; `ver` stands for the picture's version.  `next_flags(ST_CHECKED)`
    /// marks the current choice (accent frame), ST_DISABLED disables it.  EV_HOVER lets the
    /// caller show a live preview, EV_ACT applies; the label is the tooltip and the name the
    /// screen reader reads.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_image_tile(&mut self, id: u32, label: &str, sf: &alloc::rc::Rc<gfx::Surface>, ver: u32) -> u32 {
        let _src = self.src_scope();
        let flags = self.take_flags() | ST_SUBTLE;
        let selected = flags & ST_CHECKED != 0;
        let rect = self.place(self.dp(56), self.dp(66));
        let ev = self.behave(id, HB_FOCUS | if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 });
        let mut sp = Sp { flags, ..Sp::default() };
        self.sp_placed(&mut sp, ev);
        let style = self.style;
        style.button(self, &sp);
        let x = rect[0] + self.dp(2);
        let y = rect[1] + self.dp(3);
        let w = (rect[2] - self.dp(4)).max(0);
        let h = (rect[3] - self.dp(6)).max(0);
        if w > 0 && h > 0 {
            let border = self.color(if selected { T_ACCENT } else { T_DIVIDER });
            let bw = self.dp(if selected { 2 } else { 1 });
            self.cmd_rrect(x, y, w, h, 0, bw, 0xffffffff, border, border);
            let (iw, ih) = ((w - 2 * bw).max(0), (h - 2 * bw).max(0));
            if sf.w > 0 && sf.h > 0 && iw > 0 && ih > 0 {
                // the picture scaled to fit, its aspect kept
                let k = (iw as i64 * sf.h as i64).min(ih as i64 * sf.w as i64);
                let (dw, dh) = if iw as i64 * sf.h as i64 <= ih as i64 * sf.w as i64 {
                    (iw, (k / sf.w as i64) as i32)
                } else {
                    ((k / sf.h as i64) as i32, ih)
                };
                let (dx, dy) = (x + bw + (iw - dw) / 2, y + bw + (ih - dh) / 2);
                self.draw_clip(x + bw, y + bw, iw, ih);
                self.draw_image(sf, ver, dx, dy, dw, dh);
                self.draw_unclip();
            }
        }
        self.focus_ring_draw(ev);
        if selected {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_BUTTON, id, Some(label), 0);
        self.tooltip(label);
        ev
    }
}
