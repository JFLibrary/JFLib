// action_card -- a card or a list row that does one thing when it is chosen: a template on a
// start page, a recent file, a lesson
//
//   ui.action_card(id, name, &CardDesc) -> EV_*   EV_ACT when it was clicked, or Enter or Space
//                                                 while it has the keyboard
//
// CardDesc says what it shows: a title (with an icon and a badge beside it), a text below (wrapped,
// or one line cut with "…"), a caption at the far end, and above them a picture area (`picture_h`
// DIP, 0: none) with the app's drawing, without one the icon large and centred.  `name` is what
// screen readers and scripts call it (K_BUTTON); the texts follow as records of their own.
//
// next_size sets the width (natural: 200 DIP); the height follows the content, and in a row of
// cards they all take the row's height.  next_flags: ST_SUBTLE a list row (no card at rest, the
// hover fill alone), ST_CHECKED selected, ST_DISABLED no input (the texts in the disabled colour).
// The card is one Tab stop; a button of its own (remove from a list) goes beside it, not into it.

use crate::consts::*;
use crate::{DrawRef, Ui};

const PAD: u32 = 10;
const GAP: i32 = 8;

/// what an action card shows (ui.action_card); ..Default::default() for the parts it has not
#[derive(Clone, Default)]
pub struct CardDesc<'a> {
    pub title: &'a str,
    /// beside the title; with a picture area and no drawing, large in that area (0: none)
    pub icon: u32,
    /// below the title ("": none)
    pub text: &'a str,
    /// title and text stay on one line, cut with "…" (lists); else they wrap
    pub one_line: bool,
    /// a BADGE_* kind and its label, beside the title
    pub badge: Option<(u32, &'a str)>,
    /// a caption at the far end ("": none)
    pub aside: &'a str,
    /// the height of the picture area above the texts (DIP; 0: none)
    pub picture_h: i32,
    /// what it shows and the version of that drawing (render time, as cmd_custom)
    pub draw: Option<DrawRef>,
    pub draw_ver: u32,
}

impl Ui {
    /// action_card(id, name, &desc) -> EV_*: a card or list row that does one thing (the top of
    /// action_card.rs); next_size sets its width, next_flags ST_SUBTLE, ST_CHECKED, ST_DISABLED
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn action_card(&mut self, id: u32, name: &str, d: &CardDesc) -> u32 {
        let _src = self.src_scope();
        let flags = self.take_flags();
        let disabled = flags & ST_DISABLED != 0;
        if self.wp.lay.next_w == 0 {
            self.next_size(200, 0);
        }
        self.box_begin(0, BOX_V, 0);
        let r = self.box_rect();
        self.wp.ctx.last_rect = r;
        let ev = self.behave(id, HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT | if disabled { HB_DISABLED } else { 0 });
        let state = self.wp.ctx.last_state;
        let hot = !disabled && ev & EV_HOVER != 0;
        let held = hot && ev & EV_HELD != 0;
        let checked = flags & ST_CHECKED != 0;
        let subtle = flags & ST_SUBTLE != 0;
        // the surface: the theme's card, or nothing at rest for a list row
        let radius = self.dp(if subtle { 4 } else { 6 });
        let line = self.dp(1);
        let (fill, stroke) = if subtle {
            let f = self.color(if held { T_SUBTLE_PRESSED } else if hot || checked { T_SUBTLE_HOVER } else { T_SUBTLE });
            (f, f)
        } else {
            let f = self.color(if held { T_CTRL_PRESSED } else if hot || checked { T_CTRL_HOVER } else { T_CARD });
            (f, self.color(if hot || checked { T_ACCENT } else { T_CARD_STROKE }))
        };
        if r[2] > 0 && r[3] > 0 {
            self.cmd_rrect(r[0], r[1], r[2], r[3], radius, line, fill, stroke, stroke);
        }
        // the picture area: the drawing, else the icon large
        if d.picture_h > 0 {
            let area = self.place(r[2].max(1), self.dp(d.picture_h));
            if area[2] > 2 * line && area[3] > line {
                let (x, y, w, h) = (area[0] + line, area[1] + line, area[2] - 2 * line, area[3] - line);
                let top = self.color(T_LAYER);
                self.cmd_rrect(x, y, w, h, radius, 0, top, top, top);
                if let Some(dr) = &d.draw {
                    self.cmd_custom(x, y, w, h, Some(dr.clone()), 0, d.draw_ver);
                } else if d.icon != 0 {
                    let s = self.dp(32).min(h);
                    let c = self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT3 });
                    self.draw_color(c);
                    self.draw_icon(d.icon, x + (w - s) / 2, y + (h - s) / 2, s << 8);
                }
            }
        }
        // the texts
        let dis = if disabled { TX_DISABLED } else { 0 };
        let how = if d.one_line { TX_ELLIPSIS } else { TX_WRAP };
        self.next_size(-1, 0);
        self.box_begin(0, BOX_H | pad(PAD), GAP);
        if d.icon != 0 && d.picture_h == 0 {
            let s = self.dp(20);
            let at = self.place(s, s);
            let c = self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT2 });
            self.draw_color(c);
            self.draw_icon(d.icon, at[0], at[1], s << 8);
        }
        self.next_size(-1, 0);
        self.box_begin(0, BOX_V, 2);
        self.next_size(-1, 0);
        self.box_begin(0, BOX_H, GAP);
        self.next_size(-1, 0);
        self.text(TS_BODY_STRONG | how | dis, d.title);
        if let Some((kind, label)) = d.badge {
            self.badge(kind, label);
        }
        self.box_end();
        if !d.text.is_empty() {
            self.next_size(-1, 0);
            self.text(TS_CAPTION | TX_SECONDARY | how | dis, d.text);
        }
        self.box_end();
        if !d.aside.is_empty() {
            self.text(TS_CAPTION | TX_SECONDARY | dis, d.aside);
        }
        self.box_end();
        self.box_end();
        // the focus ring and the record over the whole card
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = state | if checked { SS_CHECKED as u32 } else { 0 };
        self.focus_ring_draw(ev);
        self.sem(K_BUTTON, id, Some(name), 0);
        ev
    }
}
