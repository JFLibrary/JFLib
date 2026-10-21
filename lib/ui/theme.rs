// theme -- colour tokens, accent colour and type ramp of the linked style  [src/ui/ui_theme.s]
//
//   theme_set(mode)            0 light, 1 dark, 2 follow the system (plat::theme)
//   theme_dark() -> bool       the dark theme is active
//   accent_set(rgb)            0xRRGGBB accent, 0 = the style's default (an app's accent
//                              stops the system's)
//   high_contrast() / set_contrast(Some(on) | None)   the high-contrast theme: every token
//                              from the scheme's colours (Windows: the system's; else white on
//                              black or black on white), no translucent fills; None follows
//                              the system; --contrast high|normal
//   text_scale() / set_text_scale(percent)   text sizes and line heights (not the controls)
//   sys_apply(settings)        (runtime) plat::sys_settings: theme, accent, contrast, reduced
//                              motion (anim_off), text scale, double-click and caret times,
//                              font smoothing and its subpixel order; what the app or the
//                              command line fixed stays
//   color(tok) -> argb         0xAARRGGBB of a T_* token (byte offset)
//   ts_font(ts)                select font and pixel size of a TS_* text style
//   ts_line(ts) -> px          line height of a text style
//   ts_base(ts) -> px          baseline offset from the line top
//   dp_text(dip) -> px         dp(dip) grown with a text scale above 100 % (pills around text)
//   color_mix(a, b, t)         per-channel a + (b - a) * t / 256 (t 0..256)
//
// The values come from the style (Style::tokens, accent, ts_size / ts_line / ts_font).

use super::consts::*;
use super::Ui;

/// g_theme: the T_COUNT tokens padded to a power of two (256 bytes)
pub(crate) const THEME_N: usize = 64;

/// color_mix(a, b, t): per-channel a + (b - a) * t / 256 (t 0..256), every channel
/// including alpha
pub fn color_mix(a: u32, b: u32, t: i32) -> u32 {
    let mut r = 0u32;
    for sh in (0..32).step_by(8) {
        let x = ((a >> sh) & 255) as i32;
        let y = ((b >> sh) & 255) as i32;
        let v = x + (((y - x).wrapping_mul(t)) >> 8);
        r |= (v as u32) << sh;
    }
    r
}

// the brightness of 0xRRGGBB (0..255)
fn luma(rgb: u32) -> u32 {
    ((rgb >> 16 & 255) * 299 + (rgb >> 8 & 255) * 587 + (rgb & 255) * 114) / 1000
}

/// the tokens of a high-contrast scheme: opaque fills from its colours, borders and strokes
/// in the text colour, accents in the highlight colour, links in the link colour
pub fn contrast_tokens(c: &plat::HcColors) -> [u32; T_COUNT] {
    let o = |rgb: u32| rgb | 0xFF00_0000;
    let (bg, text, hl, hlt, btn, btnt, dis, link) =
        (o(c.bg), o(c.text), o(c.highlight), o(c.highlight_text), o(c.button), o(c.button_text), o(c.disabled), o(c.link));
    let hover = color_mix(btn, hl, 64);
    let pressed = color_mix(btn, hl, 112);
    let mut t = [0u32; T_COUNT];
    let mut set = |tok: u32, v: u32| t[(tok / 4) as usize] = v;
    for tok in [T_BG, T_BG_ALT, T_LAYER, T_CARD, T_FLYOUT, T_INPUT_ACTIVE, T_CRIT_BG, T_OK_BG, T_WARN_BG, T_INFO_BG, T_ALT_FILL] {
        set(tok, bg);
    }
    for tok in [T_LAYER_STROKE, T_CARD_STROKE, T_CTRL_STROKE, T_CTRL_STROKE2, T_STRONG_STROKE, T_FLYOUT_STROKE, T_DIVIDER, T_FOCUS_OUT] {
        set(tok, text);
    }
    for tok in [T_TEXT, T_TEXT2, T_TEXT3, T_CRIT, T_OK, T_WARN, T_INFO, T_SCROLL, T_TRACK, T_KNOB, T_STRONG_FILL] {
        set(tok, text);
    }
    set(T_CTRL, btn);
    set(T_CTRL_HOVER, hover);
    set(T_CTRL_PRESSED, pressed);
    set(T_CTRL_DISABLED, btn);
    set(T_SUBTLE, btn & 0x00FF_FFFF); // transparent at rest
    set(T_SUBTLE_HOVER, hover);
    set(T_SUBTLE_PRESSED, pressed);
    set(T_SUBTLE_DISABLED, btn & 0x00FF_FFFF);
    set(T_TEXT_DISABLED, dis);
    set(T_ACCENT, hl);
    set(T_ACCENT_HOVER, hl);
    set(T_ACCENT_PRESSED, hl);
    set(T_ACCENT_DISABLED, dis);
    set(T_ON_ACCENT, hlt);
    set(T_ON_ACCENT_DIS, bg);
    set(T_ACCENT_TEXT, link);
    set(T_FOCUS_IN, bg);
    set(T_SMOKE, 0xA000_0000);
    set(T_CLOSE_HOVER, hl);
    set(T_SHADOW, 0);
    if btnt != text {
        set(T_KNOB, btnt);
    }
    t
}

impl Ui {
    /// ui_theme_set(mode): 0 light, 1 dark, 2 the system setting (unknown: dark)
    pub fn theme_set(&mut self, mode: u32) {
        let mut m = mode;
        self.theme_sys = m == 2;
        if m == 2 {
            let t = plat::theme();
            m = if t < 0 { 1 } else { t as u32 };
        }
        let before = self.theme_dark;
        self.theme_base_dark = m & 1 != 0;
        self.theme_build();
        if before != self.theme_dark { self.theme_invalidate(); }
    }

    /// ui_theme_dark: the dark theme is active
    pub fn theme_dark(&self) -> bool {
        self.theme_dark
    }

    /// ui_accent_set(0xRRGGBB), 0 = the style's default; the system's accent no longer applies
    pub fn accent_set(&mut self, rgb: u32) {
        self.accent = rgb & 0xFF_FFFF;
        self.accent_sys = false;
        self.theme_build();
    }

    /// the high-contrast theme is active
    pub fn high_contrast(&self) -> bool {
        self.high_contrast
    }

    /// high contrast on (Some(true)), off (Some(false)) or as the system says (None)
    pub fn set_contrast(&mut self, on: Option<bool>) {
        self.contrast_mode = match on {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        self.high_contrast = match on {
            Some(v) => v,
            None => self.high_contrast,
        };
        self.theme_build();
    }

    /// the text scale in percent (100 = the style's sizes)
    pub fn text_scale(&self) -> u32 {
        self.text_scale
    }

    /// the system's font rendering: (smoothing, plat::FONT_HINT_*, plat::FONT_RGBA_*), None
    /// where the system says nothing; smoothing off (ui.text.antialias()) and the subpixel
    /// order (ui.text.subpixel(): subpixel smoothing on the window's framebuffer) are applied,
    /// hinting is not
    pub fn font_prefs(&self) -> (Option<bool>, Option<u8>, Option<u8>) {
        self.font_prefs
    }

    /// scale text sizes and line heights (50..300 %); the system's scale no longer applies
    pub fn set_text_scale(&mut self, percent: u32) {
        self.text_scale_forced = true;
        self.text_scale_to(percent);
    }

    fn text_scale_to(&mut self, percent: u32) {
        let p = percent.clamp(50, 300);
        if p != self.text_scale {
            self.text_scale = p;
            self.theme_invalidate();
            self.repass = true;
        }
    }

    /// (runtime) the system's settings, except what the app or the command line fixed
    pub(crate) fn sys_apply(&mut self, s: plat::SysSettings) {
        if self.theme_sys {
            if let Some(d) = s.dark {
                self.theme_base_dark = d;
            }
        }
        if self.accent_sys {
            self.accent = s.accent.unwrap_or(0) & 0xFF_FFFF;
        }
        self.high_contrast = match self.contrast_mode {
            1 => true,
            2 => false,
            _ => s.high_contrast,
        };
        self.hc_sys = s.contrast_colors;
        if !self.anim_forced {
            self.anim_off = s.reduce_motion;
        }
        if !self.text_scale_forced {
            self.text_scale_to(s.text_scale);
        }
        self.dbl_ms = s.double_click_ms;
        self.blink_ms = s.caret_blink_ms;
        // font smoothing off: black-and-white glyphs; a subpixel order: subpixel smoothing on
        // opaque targets (text/lcd.rs; headless only through jfgui_FONT_RGBA, so shots stay
        // the same everywhere); hinting is kept for the app (font_prefs)
        let aa = s.font_aa != Some(false);
        let order = if aa { s.font_rgba.unwrap_or(0) } else { 0 }; // FONT_RGBA_* = text::SUBPIXEL_*
        if self.text.antialias() != aa || self.text.subpixel() != order {
            self.text.set_antialias(aa);
            self.text.set_subpixel(order);
            self.theme_invalidate(); // every pixel of text changes: redraw everything
        }
        self.font_prefs = (s.font_aa, s.font_hinting, s.font_rgba);
        self.theme_build();
    }

    /// ui_theme_build: copy the table for the current mode and fill in the accent tokens (high
    /// contrast: the scheme's tokens)
    pub(crate) fn theme_build(&mut self) {
        let before = self.theme_dark;
        let t = if self.high_contrast {
            // the system's scheme, else white on black (dark) or black on white (light)
            let dark = self.theme_base_dark;
            let c = self.hc_sys.unwrap_or(if dark { plat::HcColors::BLACK } else { plat::HcColors::WHITE });
            self.theme_dark = luma(c.bg) < 128;
            contrast_tokens(&c)
        } else {
            self.theme_dark = self.theme_base_dark;
            let st = self.style;
            let mut t = st.tokens(self.theme_dark);
            st.accent(&mut t, self.theme_dark, self.accent); // the style derives the accent tokens
            t
        };
        let changed = self.theme[..T_COUNT] != t || before != self.theme_dark;
        self.theme[..T_COUNT].copy_from_slice(&t);
        if changed { self.theme_invalidate(); }
    }

    // agent_core: style icon colours may change without changing Cmd::Icon's
    // foreground. Invalidate the live window and every saved window together.
    fn theme_invalidate(&mut self) {
        self.dirty_force_full();
        for window in &mut self.m.win.recs {
            if let Some(parts) = &mut window.save { parts.dirty.full = true; }
        }
        self.need = true;
        self.need_render = true;
    }

    /// ui_color(tok): 0xAARRGGBB of a token (byte offset; any value reads inside the table)
    pub fn color(&self, tok: u32) -> u32 {
        self.theme[((tok & 252) / 4) as usize]
    }

    /// ui_ts_font(ts): select the font and pixel size (size * dpi / 96, 24.8) of a style
    pub fn ts_font(&mut self, ts: u32) {
        let ts = if (ts & 7) < TS_COUNT { ts & 7 } else { TS_BODY } as usize;
        let size = (self.style.ts_size_fx()[ts] as u32).wrapping_mul(self.wp.rt.dpi as u32) / 96;
        let size = if self.text_scale == 100 { size } else { size * self.text_scale / 100 };
        self.text.set_font(self.style.ts_font()[ts] as u32, size as i32);
    }

    /// ui_ts_line(ts): line height of a text style in px
    pub fn ts_line(&self, ts: u32) -> i32 {
        let ts = if (ts & 7) < TS_COUNT { ts & 7 } else { TS_BODY } as usize;
        let dip = self.style.ts_line()[ts] as i32;
        self.dp(if self.text_scale == 100 { dip } else { (dip * self.text_scale as i32 + 50) / 100 })
    }

    /// dp_text(dip): DIP -> px, grown with a text scale above 100 % (the height of parts
    /// around one line of text, such as badge pills)
    pub fn dp_text(&self, dip: i32) -> i32 {
        self.dp(if self.text_scale <= 100 { dip } else { (dip * self.text_scale as i32 + 50) / 100 })
    }

    /// ui_ts_base(ts): baseline offset from the top of the line box (px; selects the font)
    pub fn ts_base(&mut self, ts: u32) -> i32 {
        self.ts_font(ts);
        let line = self.ts_line(ts);
        let asc = self.text.ascent();
        let desc = self.text.descent();
        ((line - (asc + desc)) >> 1) + asc
    }
}
