// Ribbon groups and typography tiles. Additive native UI component by office_ribbon;
// no legacy counterpart. Uses normal focus, semantics, popups and style tokens.
use crate::popup::PPF_FOCUS;
use crate::{
    Sp, Ui,
    consts::*,
    keys::{IC_MORE, IC_OPEN_IN_NEW_WINDOW, VK_DOWN, VK_F4},
};
use alloc::vec::Vec;

/// Explicit typography for a document sample, independent of the application's theme.
/// Sizes and baseline displacement are 24.8 DIP; positive displacement moves downwards.
#[derive(Clone, Copy, Debug)]
pub struct RibbonSample {
    pub font: u32,
    pub size: i32,
    pub color: u32,
    pub highlight: Option<u32>,
    pub baseline_shift: i32,
    pub underline: bool,
    pub strike: bool,
}

/// Style-owned ribbon geometry in DIP. A scrolling ribbon keeps every group's
/// original arrangement; a collapsing ribbon can use the compact/flyout layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RibbonMetrics {
    pub content_height: i32,
    pub caption_height: i32,
    pub group_gap: i32,
    pub content_gap: i32,
    /// Zero uses the style's existing uniform `ribbon_padding`.
    pub inset_x: i32,
    pub inset_y: i32,
    pub collapse_groups: bool,
}
impl Default for RibbonMetrics {
    fn default() -> Self {
        Self {
            content_height: 66,
            caption_height: 18,
            group_gap: 8,
            content_gap: 2,
            inset_x: 0,
            inset_y: 0,
            collapse_groups: true,
        }
    }
}

/// One ribbon group. Reserve `id..=id + 3` for its box, overflow and popup.
#[derive(Clone, Copy)]
pub struct RibbonGroup<'a> {
    /// Stable base ID, unique across the window.
    pub id: u32,
    /// Visible group caption and accessible overflow name.
    pub label: &'a str,
    /// Preferred group width in DIP.
    pub width: i32,
    /// Smallest width that still displays all controls, in DIP.
    pub compact_width: i32,
}
impl<'a> RibbonGroup<'a> {
    /// Construct a group with a preferred and compact content width.
    pub const fn new(id: u32, label: &'a str, width: i32, compact_width: i32) -> Self {
        Self {
            id,
            label,
            width,
            compact_width,
        }
    }
}

/// Presentation of an open ribbon group; pass it to `ribbon_group_end`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RibbonGroupMode {
    /// Preferred or compact inline width in DIP.
    Inline(i32),
    /// Group shown in a popup at its preferred width in DIP.
    Flyout(i32),
}
impl RibbonGroupMode {
    /// Actual width available to the controls, in DIP.
    pub fn width(self) -> i32 {
        match self {
            Self::Inline(w) | Self::Flyout(w) => w,
        }
    }
}

/// Per-frame ribbon layout: shrink groups before moving them into labeled flyouts.
pub struct RibbonLayout {
    widths: Vec<i32>,
    collapsed: Vec<bool>,
}
impl RibbonLayout {
    /// Compute group presentation for an available inner width in DIP.
    /// Groups collapse from right to left; extremely narrow windows can scroll.
    pub fn new(available_width: i32, groups: &[RibbonGroup<'_>]) -> Self {
        Self::with_metrics(available_width, groups, RibbonMetrics::default())
    }
    /// Compute a layout using the active style's sizing and overflow policy.
    pub fn with_metrics(
        available_width: i32,
        groups: &[RibbonGroup<'_>],
        metrics: RibbonMetrics,
    ) -> Self {
        let mut widths: Vec<i32> = groups.iter().map(|g| g.width.max(48)).collect();
        let mut collapsed = alloc::vec![false; groups.len()];
        if !metrics.collapse_groups {
            return Self { widths, collapsed };
        }
        let gaps = groups.len().saturating_sub(1) as i64 * i64::from(metrics.group_gap.max(0));
        let mut total = widths.iter().map(|&w| i64::from(w)).sum::<i64>() + gaps;
        for (i, g) in groups.iter().enumerate().rev() {
            if total <= i64::from(available_width) {
                break;
            }
            let smaller = g.compact_width.clamp(48, widths[i]);
            total -= i64::from(widths[i] - smaller);
            widths[i] = smaller;
        }
        for i in (0..groups.len()).rev() {
            if total <= i64::from(available_width) {
                break;
            }
            let smaller = widths[i].min(96);
            total -= i64::from(widths[i] - smaller);
            widths[i] = smaller;
            collapsed[i] = true;
        }
        Self { widths, collapsed }
    }
    /// Whether a group is reachable through its labeled overflow button.
    pub fn is_collapsed(&self, index: usize) -> bool {
        self.collapsed.get(index).copied().unwrap_or(false)
    }
    /// Width occupied by a group in the ribbon, in DIP.
    pub fn group_width(&self, index: usize) -> Option<i32> {
        self.widths.get(index).copied()
    }
}

impl Ui {
    /// Start a style-sized ribbon (110 DIP by default). Pair with `ribbon_end`.
    /// All groups remain keyboard reachable; a scroll strip is the final fallback.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_begin(&mut self, id: u32, groups: &[RibbonGroup<'_>]) -> RibbonLayout {
        let _src = self.src_scope();
        let m = self.style.ribbon_metrics();
        let py = if m.inset_y > 0 {
            m.inset_y as u32
        } else {
            self.style.ribbon_padding()
        };
        self.next_size(-1, self.style.ribbon_height());
        self.scroll_begin(
            id,
            SC_H | BOX_H | BOX_LAYER | pad(py) | if m.collapse_groups { 0 } else { SC_NOBAR },
        );
        if m.inset_x > py as i32 {
            self.spacer(m.inset_x - py as i32);
        }
        let available = self.px2dip(self.box_rect()[2]);
        RibbonLayout::with_metrics(available, groups, m)
    }

    /// Begin a group's controls when visible, or render its overflow button.
    /// Only call `ribbon_group_end` when this returns `Some`.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_group_begin(
        &mut self,
        layout: &RibbonLayout,
        index: usize,
        group: &RibbonGroup<'_>,
    ) -> Option<RibbonGroupMode> {
        let _src = self.src_scope();
        let width = layout.group_width(index)?;
        let m = self.style.ribbon_metrics();
        let gap = if m.collapse_groups { 4 } else { 0 };
        let height = m.content_height + m.caption_height + gap;
        if index != 0 {
            self.spacer(m.group_gap);
        }
        if layout.is_collapsed(index) {
            self.next_size(width, height);
            self.box_begin(group.id, BOX_V | BOX_FIXED, gap);
            self.next_size(-1, m.content_height);
            if self.ribbon_button(group.id + 1, group.label, IC_MORE, ST_LARGE) & EV_ACT != 0 {
                self.popup_open(group.id + 2);
            }
            let anchor = self.last_rect();
            self.next_size(-1, m.caption_height);
            self.text(
                TS_CAPTION | TX_SECONDARY | TX_CENTER | TX_ELLIPSIS,
                crate::i18n::tr("Weitere Befehle"),
            );
            self.box_end();
            self.wp.ctx.last_rect = anchor;
            if !self.flyout_begin(group.id + 2, 0) {
                self.flyout_end();
                return None;
            }
            self.next_size(group.width.max(48), m.content_height);
            self.box_begin(group.id + 3, BOX_V | BOX_FIXED, m.content_gap);
            Some(RibbonGroupMode::Flyout(group.width.max(48)))
        } else {
            // A resize can expand an open overflow group. Remove its old popup
            // immediately so no invisible popup layer intercepts input.
            if self.popup_is_open(group.id + 2) {
                self.popup_close();
            }
            self.next_size(width, height);
            self.box_begin(group.id, BOX_V | BOX_FIXED, gap);
            let r = self.last_rect();
            let c = self.color(T_DIVIDER);
            self.draw_color(c);
            if m.collapse_groups || index + 1 < layout.widths.len() {
                self.draw_rect(
                    r[0] + r[2] + self.dp((m.group_gap - 2).max(0) / 2),
                    r[1] + self.dp(2),
                    self.hair(),
                    r[3] - self.dp(4),
                );
            }
            self.next_size(-1, m.content_height);
            self.box_begin(group.id + 3, BOX_V | BOX_FIXED, m.content_gap);
            Some(RibbonGroupMode::Inline(width))
        }
    }

    /// Finish controls and draw the inline group caption; closes overflow when appropriate.
    /// Flyouts already have their named opener, so no trailing caption is painted
    /// over a nested action menu emitted while building their controls.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_group_end(&mut self, mode: RibbonGroupMode, label: &str) {
        let _src = self.src_scope();
        self.box_end();
        match mode {
            RibbonGroupMode::Inline(_) => {
                self.next_size(-1, self.style.ribbon_metrics().caption_height);
                self.text(TS_CAPTION | TX_SECONDARY | TX_CENTER | TX_ELLIPSIS, label);
                self.box_end();
            }
            RibbonGroupMode::Flyout(_) => self.flyout_end(),
        }
    }

    /// Finish a group with a separately focusable dialog-launch button in its caption.
    /// `id` is an additional unique widget ID; `action_label` is its full semantic name.
    /// Activation is returned to the app, which opens its existing dialog. Disabled
    /// launchers retain their label and cannot be invoked through keyboard or agents.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_group_end_launcher(
        &mut self,
        mode: RibbonGroupMode,
        label: &str,
        id: u32,
        action_label: &str,
        flags: u32,
    ) -> u32 {
        let _src = self.src_scope();
        self.box_end();
        let caption = self.style.ribbon_metrics().caption_height;
        self.next_size(-1, caption);
        self.box_begin(0, BOX_H, 2);
        self.next_size(-1, caption);
        self.text(TS_CAPTION | TX_SECONDARY | TX_CENTER | TX_ELLIPSIS, label);
        self.next_size(caption, caption);
        let ev = self.ribbon_button(
            id,
            action_label,
            IC_OPEN_IN_NEW_WINDOW,
            flags | ST_ICON_ONLY,
        );
        self.box_end();
        match mode {
            RibbonGroupMode::Inline(_) => self.box_end(),
            RibbonGroupMode::Flyout(_) => self.flyout_end(),
        }
        ev
    }

    /// Finish the ribbon begun with `ribbon_begin`.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_end(&mut self) {
        let _src = self.src_scope();
        let id = self.box_id();
        let m = self.style.ribbon_metrics();
        let py = if m.inset_y > 0 {
            m.inset_y
        } else {
            self.style.ribbon_padding() as i32
        };
        if m.inset_x > py {
            self.spacer(m.inset_x - py);
        }
        self.scroll_end();
        if !m.collapse_groups {
            let r = self.last_rect();
            let (extent, _) = self.scroll_extent(id);
            let (offset, _) = self.scroll_pos(id);
            let viewport = self.px2dip(r[2]);
            for (part, shown, x, icon, label, target) in [
                (
                    4,
                    offset > 0,
                    r[0],
                    crate::keys::IC_CHEV_LEFT,
                    "Vorherige Menübandbefehle",
                    (offset - 160).max(0),
                ),
                (
                    5,
                    offset + viewport < extent,
                    r[0] + r[2] - self.dp(18),
                    crate::keys::IC_CHEV_RIGHT,
                    "Weitere Menübandbefehle",
                    (offset + 160).min((extent - viewport).max(0)),
                ),
            ] {
                if !shown {
                    continue;
                }
                self.draw_color(self.color(T_LAYER));
                self.draw_rect(x, r[1], self.dp(18), r[3]);
                self.next_rect_px(x, r[1], self.dp(18), r[3]);
                if self.ribbon_button(
                    id.wrapping_mul(8).wrapping_add(part) | 0x8000_0000,
                    label,
                    icon,
                    ST_ICON_ONLY,
                ) & EV_ACT
                    != 0
                {
                    self.scroll_to(id, target, 0);
                }
            }
            self.wp.ctx.last_rect = r;
        }
    }

    /// Natural action width in DIP, including the style's icon, label and menu
    /// spacing. Measuring neither consumes next_* hints nor creates a widget.
    /// Use it when composing translated ribbon columns before ribbon_begin.
    pub fn ribbon_button_width(&mut self, label: &str, icon: u32, flags: u32) -> i32 {
        let saved = self.text.get_font();
        let mut sp = Sp {
            flags: flags | ST_SUBTLE | ST_COMPACT,
            icon,
            label: Some(label),
            ..Sp::default()
        };
        let st = self.style;
        let (width, _) = st.button_size(self, &mut sp);
        self.text.set_font(saved.0, saved.1);
        // Round up so integer DIP columns never clip a fractional-width glyph.
        let dpi = self.dpi().max(1) as i64;
        ((width.max(0) as i64 * 96 + dpi - 1) / dpi).min(i32::MAX as i64) as i32
    }

    /// Compact ribbon action with clipped drawing, native focus and a full tooltip.
    /// `ST_ICON_ONLY`, `ST_LARGE`, `ST_CHECKED` and `ST_DISABLED` are supported.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_button(&mut self, id: u32, label: &str, icon: u32, flags: u32) -> u32 {
        let _src = self.src_scope();
        let flags = flags | self.take_flags() | ST_SUBTLE | ST_COMPACT;
        let st = self.style;
        let mut sp = Sp {
            flags,
            icon,
            label: Some(label),
            ..Sp::default()
        };
        let (w, h) = st.button_size(self, &mut sp);
        let r = self.place(w, h);
        let ev = self.behave(
            id,
            HB_FOCUS
                | if flags & ST_DISABLED != 0 {
                    HB_DISABLED
                } else {
                    0
                },
        );
        self.sp_placed(&mut sp, ev);
        self.draw_clip(r[0], r[1], r[2], r[3]);
        st.button(self, &sp);
        self.draw_unclip();
        self.focus_ring_draw(ev);
        if flags & ST_CHECKED != 0 {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_BUTTON, id, Some(label), 0);
        self.tooltip(label);
        ev
    }

    /// Large ribbon action: the style draws its icon above a visible caption.
    /// It is one normal focusable, inspectable button. Use `next_size` to override
    /// its natural style size; checked/disabled flags keep their ordinary meaning.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_large_button(&mut self, id: u32, label: &str, icon: u32, flags: u32) -> u32 {
        let _src = self.src_scope();
        let flags = flags | self.take_flags();
        self.ribbon_button(id, label, icon, (flags | ST_LARGE) & !ST_ICON_ONLY)
    }

    /// Ribbon-styled menu opener. `id` is also the popup identifier. Its natural
    /// dimensions, icon, menu indicator and enabled state come from the style.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_menu_button(&mut self, id: u32, label: &str, icon: u32, flags: u32) -> u32 {
        let _src = self.src_scope();
        let flags = flags | self.take_flags();
        let ev = self.ribbon_button(id, label, icon, flags | ST_MENU);
        if flags & ST_DISABLED == 0 && (ev & EV_ACT != 0 || ev & EV_FOCUS != 0 && self.open_key()) {
            self.popup_open(id);
            if self.wp.ctx.focus_visible {
                self.pp_flag(id, PPF_FOCUS);
            }
        }
        if self.popup_is_open(id) {
            self.wp.ctx.last_state |= SS_OPEN as u32;
            if let Some(semantic) = self.sems.last_mut() {
                if semantic.id == id {
                    semantic.state |= SS_OPEN;
                }
            }
        }
        ev | if self.popup_is_open(id) { EV_OPEN } else { 0 }
    }

    /// Large split action with an icon hit area above its menu caption. Both
    /// parts remain independently focusable and discoverable through semantics.
    /// Pair the `menu_id` with a normal flyout. Alt+Down/F4 opens the caption menu.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_stacked_split_button(
        &mut self,
        id: u32,
        menu_id: u32,
        label: &str,
        menu_label: &str,
        icon: u32,
        flags: u32,
    ) -> u32 {
        let _src = self.src_scope();
        let flags = flags | self.take_flags() | ST_LARGE | ST_COMPACT | ST_SUBTLE | ST_MENU;
        let st = self.style;
        let mut sp = Sp {
            flags,
            icon,
            label: Some(label),
            ..Sp::default()
        };
        let (w, h) = st.button_size(self, &mut sp);
        let whole = self.place(w, h);
        let top = self.dp(40).min(whole[3]);
        let hit_flags = HB_FOCUS
            | if flags & ST_DISABLED != 0 {
                HB_DISABLED
            } else {
                0
            };
        self.wp.ctx.last_rect = [whole[0], whole[1], whole[2], top];
        let ev = self.behave(id, hit_flags);
        self.sp_placed(&mut sp, ev);
        self.sem(K_BUTTON, id, Some(label), 0);
        self.focus_ring_draw(ev);
        self.tooltip(label);
        self.wp.ctx.last_rect = [whole[0], whole[1] + top, whole[2], whole[3] - top];
        let menu = self.behave(menu_id, hit_flags);
        let keyboard = (ev | menu) & EV_FOCUS != 0 && self.open_key();
        if flags & ST_DISABLED == 0 && (menu & EV_ACT != 0 || keyboard) {
            if keyboard {
                self.focus(menu_id);
            }
            self.popup_open(menu_id);
            if self.wp.ctx.focus_visible {
                self.pp_flag(menu_id, PPF_FOCUS);
            }
        }
        if self.popup_is_open(menu_id) {
            self.wp.ctx.last_state |= SS_OPEN as u32;
        }
        self.sem(K_BUTTON, menu_id, Some(menu_label), 0);
        self.focus_ring_draw(menu);
        self.tooltip(menu_label);
        if menu & EV_HOVER != 0 {
            self.sp_placed(&mut sp, menu);
        }
        sp.place(whole);
        self.draw_clip(whole[0], whole[1], whole[2], whole[3]);
        st.button(self, &sp);
        self.draw_unclip();
        self.wp.ctx.last_rect = whole;
        ev | if self.popup_is_open(menu_id) {
            EV_OPEN
        } else {
            0
        }
    }

    /// A large primary action beside an independently focusable dropdown.
    /// `id` identifies the primary action; `menu_id` identifies both the dropdown
    /// button and its popup. Both IDs must be unique. `menu_label` is a full semantic
    /// name, not an unlabeled chevron. Pair with `flyout_begin(menu_id, ..)` and
    /// `flyout_end` immediately afterwards to provide actual secondary actions.
    /// Returns primary EV_* flags plus EV_OPEN while the dropdown popup is open.
    /// Tab reaches both parts; Enter/Space activate the focused part. Alt+Down or
    /// unbound F4 on the main part forwards to the existing dropdown keyboard path.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_split_button(
        &mut self,
        id: u32,
        menu_id: u32,
        label: &str,
        menu_label: &str,
        icon: u32,
        flags: u32,
    ) -> u32 {
        let _src = self.src_scope();
        let flags = flags | self.take_flags();
        let st = self.style;
        let mut sp = Sp {
            flags: (flags | ST_LARGE | ST_COMPACT | ST_SUBTLE) & !ST_ICON_ONLY,
            icon,
            label: Some(label),
            ..Sp::default()
        };
        let (width, height) = st.button_size(self, &mut sp);
        let arrow = self.dp(20);
        let whole = self.place(width + arrow, height);
        let arrow = arrow.min(whole[2].max(0));
        let main = [whole[0], whole[1], (whole[2] - arrow).max(0), whole[3]];
        self.next_rect_px(main[0], main[1], main[2], main[3]);
        let ev = self.ribbon_large_button(id, label, icon, flags);
        if ev & EV_FOCUS != 0 && flags & ST_DISABLED == 0 {
            let key = self.key_first();
            let unbound_f4 = key == VK_F4
                && !self
                    .desc
                    .cmds
                    .iter()
                    .take_while(|command| command.id != 0)
                    .any(|command| command.key as u32 == VK_F4 && command.mods == 0);
            if key == (VK_DOWN | (MOD_ALT << 16)) || unbound_f4 {
                self.focus(menu_id);
            }
        }
        self.next_rect_px(whole[0] + main[2], whole[1], arrow, whole[3]);
        self.next_flags((flags | ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY) & !ST_LARGE);
        let menu = self.dropdown_button(menu_id, menu_label);
        // Popup placement and caller tooltips use the whole combined rectangle;
        // semantic and focus records still retain each part's own hit rectangle.
        self.wp.ctx.last_rect = whole;
        ev | (menu & EV_OPEN)
    }

    /// Begin an inset horizontal gallery on an input-colored surface. The caller
    /// supplies its size and tiles; finish with `box_end`. The whole frame is painted
    /// before applying the three-DIP tile inset, including in dark themes.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_gallery_begin(&mut self, id: u32) {
        let _src = self.src_scope();
        self.box_begin(id, BOX_H | BOX_FIXED | pad(3), 2);
        let r = self.box_rect();
        let inset = self.dp(3);
        let stroke = self.color(T_LAYER_STROKE);
        self.cmd_rrect(r[0] - inset, r[1] - inset, r[2] + 2 * inset, r[3] + 2 * inset,
            self.dp(4), self.dp(1), self.color(T_INPUT_ACTIVE), stroke, stroke);
    }

    /// Typography preview tile for a style gallery; selection is exposed semantically.
    /// `typography` is a `TS_*` style; use `next_size` to set a compact gallery width.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_tile(
        &mut self,
        id: u32,
        label: &str,
        preview: &str,
        typography: u32,
        selected: bool,
    ) -> u32 {
        let _src = self.src_scope();
        self.ribbon_font_tile(id, label, preview, typography, selected, None)
    }

    /// A gallery tile with an optional document font, 24.8-DIP size and ARGB ink.
    /// The surrounding control follows the application theme; the sample stays document-colored.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_font_tile(&mut self, id: u32, label: &str, preview: &str, typography: u32,
        selected: bool, face: Option<(u32, i32, u32)>) -> u32 {
        let _src = self.src_scope();
        self.ribbon_sample_tile(id, label, preview, typography, selected, face.map(|(font, size, color)|
            RibbonSample { font, size, color, highlight: None, baseline_shift: 0, underline: false, strike: false }))
    }
    /// Style-gallery tile with explicit highlight, baseline and text decorations.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn ribbon_sample_tile(&mut self, id: u32, label: &str, preview: &str, typography: u32,
        selected: bool, sample: Option<RibbonSample>) -> u32 {
        let _src = self.src_scope();
        let flags = self.take_flags() | ST_SUBTLE | if selected { ST_CHECKED } else { 0 };
        let r = self.place(self.dp(106), self.dp(62));
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
        let st = self.style;
        st.button(self, &sp);
        if let Some(sample) = sample.or_else(|| st.ribbon_preview(typography).map(|(font, size, color)|
            RibbonSample { font, size, color, highlight: None, baseline_shift: 0, underline: false, strike: false })) {
            // These are document samples. The palette remains independent of
            // dark chrome and disabled commands do not gray out the preview.
            self.draw_color(0xffffffff);
            self.draw_rect(
                r[0] + self.dp(3),
                r[1] + self.dp(4),
                (r[2] - self.dp(6)).max(0),
                self.dp(32).min((r[3] - self.dp(4)).max(0)),
            );
            self.draw_clip(r[0] + self.dp(3), r[1], (r[2] - self.dp(6)).max(0), r[3]);
            self.text.set_font(sample.font, self.dp_fx(sample.size));
            let asc = self.text.ascent();
            let desc = self.text.descent();
            let x = r[0] + self.dp(4);
            let baseline = r[1] + self.dp(4) + (self.dp(34) - asc - desc) / 2 + asc
                + self.dp_fx(sample.baseline_shift) / 256;
            let width = self.text.width(preview);
            if let Some(color) = sample.highlight {
                self.draw_color(color);
                self.draw_rect(x, baseline - asc, width, asc + desc);
            }
            self.emit_text(
                x,
                baseline,
                preview,
                sample.color,
            );
            self.draw_color(sample.color);
            let thickness = (self.dp_fx(sample.size) / 256 / 16).max(1);
            if sample.underline { self.draw_rect(x, baseline + thickness, width, thickness); }
            if sample.strike { self.draw_rect(x, baseline - (asc / 3).max(1), width, thickness); }
            self.text.set_font(FONT_REG, self.dp_fx(2688)); // 10.5 DIP
            let room = (r[2] - self.dp(6)).max(0);
            let truncated = if self.text.width(label) > room {
                Some(self.ellipsize(label.as_bytes(), room).0)
            } else {
                None
            };
            let caption = truncated.as_deref().unwrap_or(label);
            let w = self.text.width(caption);
            let baseline = r[1] + r[3] - self.dp(9)
                + (self.text.ascent() - self.text.descent()) / 2;
            self.emit_text(
                r[0] + ((r[2] - w) / 2).max(self.dp(3)),
                baseline,
                caption,
                self.color(T_TEXT),
            );
            self.draw_unclip();
            self.focus_ring_draw(ev);
            if selected {
                self.wp.ctx.last_state |= SS_CHECKED as u32;
            }
            self.sem(K_BUTTON, id, Some(label), i32::from(selected));
            self.tooltip(label);
            return ev;
        }
        self.draw_clip(r[0] + self.dp(6), r[1], (r[2] - self.dp(12)).max(0), r[3]);
        let c = self.color(if sp.state == 3 {
            T_TEXT_DISABLED
        } else if typography == TS_SUBTITLE {
            T_ACCENT_TEXT
        } else {
            T_TEXT
        });
        let base = self.ts_base(typography);
        let line = self.ts_line(typography);
        self.emit_text(
            r[0] + self.dp(8),
            r[1] + self.dp(4) + (self.dp(34) - line) / 2 + base,
            preview,
            c,
        );
        let base = self.ts_base(TS_CAPTION);
        let c = self.color(if sp.state == 3 {
            T_TEXT_DISABLED
        } else {
            T_TEXT2
        });
        self.emit_text(r[0] + self.dp(8), r[1] + self.dp(42) + base, label, c);
        self.draw_unclip();
        self.focus_ring_draw(ev);
        if selected {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_BUTTON, id, Some(label), i32::from(selected));
        self.tooltip(label);
        ev
    }
}
