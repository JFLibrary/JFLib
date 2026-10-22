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
//   set_reduced_motion(Some(on) | None)   reduced motion (anim_off) forced on or off, or as
//                              the system says (None); --anim off|on
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
//   luminance(rgb) / contrast_ratio(a, b)   WCAG 2 relative luminance (0..1) of 0xRRGGBB and
//                              the contrast of two colours (1..21; text wants 4.5, graphics 3)
// App themes (theme_app.rs: Theme, its file format and errors) on top of the style:
//   set_theme(&theme) -> Result   from the next frame on; a font that cannot be loaded is an
//                              error and the previous theme stays; clear_theme() the style's
//                              own look again; app_theme() -> Option<&Theme>;
//                              load_theme(path) = Theme::load + set_theme
//   The theme layer: theme_build (colours: the theme's tokens over the style's, its accent
//   through the style's ramp), cmd_rrect / cmd_shadow (look_radius: corners that are not pills),
//   box_begin / gridbox_begin (dp_space: padding and gaps), the size calls of buttons, text
//   boxes, combo boxes, drop-down and split buttons (ctrl_size), ts_font / ts_line (sizes,
//   line heights, weights, the family's faces).  Without a theme every value is the style's.
// The run-time style switch (styles linked side by side, build_rs.py: every style_NAME the
// app's sources name):
//   styles_add(&[&style_kde::STYLE, ..])   style_names() -> names   style_name() -> current
//   set_style(name) -> bool    the style's fonts replace ids 0..3 (Text::set_embedded), the
//                              tokens are built again, everything is redrawn and laid out
// look.serial counts the changes of theme and style: caches of laid-out text compare it.
//
// The values come from the style (Style::tokens, accent, ts_size / ts_line / ts_font).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::consts::*;
use super::{FontSel, Style, Ui};

#[path = "theme_app.rs"]
mod app;
pub use app::{token_index, Radius, TextStyle, Theme, ThemeError, THEME_CONTRAST, THEME_DARK, THEME_LIGHT, TOKEN_NAMES, TS_NAMES};
pub(crate) use app::parse_color;

/// the app's theme, resolved for the drawing calls, and the styles set_style can switch to
pub(crate) struct Look {
    /// the active theme (None: the style's look)
    pub(crate) theme: Option<Box<Theme>>,
    /// corners: 0 the style's, 1 rad DIP, 2 rad percent of the style's
    pub(crate) rad_mode: u8,
    pub(crate) rad: i32,
    /// box padding and gaps in percent
    pub(crate) spacing: i32,
    /// standard control height in DIP (0: the style's)
    pub(crate) ctrl_h: i32,
    /// the theme changes text styles; the font ids it uses for FONT_REG / _SEMI / _BOLD
    pub(crate) text: bool,
    pub(crate) fonts: [u32; 3],
    /// font files themes loaded (path, id): a theme applied again reuses them
    files: Vec<(String, u32)>,
    /// the styles set_style can switch to (the app's own first)
    styles: Vec<&'static dyn Style>,
    /// changes of theme and style so far
    pub(crate) serial: u32,
}

impl Look {
    pub(crate) fn new(style: &'static dyn Style) -> Look {
        Look {
            theme: None,
            rad_mode: 0,
            rad: 0,
            spacing: 100,
            ctrl_h: 0,
            text: false,
            fonts: [FONT_REG, FONT_SEMI, FONT_BOLD],
            files: Vec::new(),
            styles: alloc::vec![style],
            serial: 0,
        }
    }
}

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

// sRGB byte -> linear light * 65535 (IEC 61966-2-1)
static SRGB_LIN: [u16; 256] = [
    0, 20, 40, 60, 80, 99, 119, 139, 159, 179, 199, 219, 241, 264, 288, 313,
    340, 367, 396, 427, 458, 491, 526, 562, 599, 637, 677, 718, 761, 805, 851, 898,
    947, 997, 1048, 1101, 1156, 1212, 1270, 1330, 1391, 1453, 1517, 1583, 1651, 1720, 1790, 1863,
    1937, 2013, 2090, 2170, 2250, 2333, 2418, 2504, 2592, 2681, 2773, 2866, 2961, 3058, 3157, 3258,
    3360, 3464, 3570, 3678, 3788, 3900, 4014, 4129, 4247, 4366, 4488, 4611, 4736, 4864, 4993, 5124,
    5257, 5392, 5530, 5669, 5810, 5953, 6099, 6246, 6395, 6547, 6700, 6856, 7014, 7174, 7335, 7500,
    7666, 7834, 8004, 8177, 8352, 8528, 8708, 8889, 9072, 9258, 9445, 9635, 9828, 10022, 10219, 10417,
    10619, 10822, 11028, 11235, 11446, 11658, 11873, 12090, 12309, 12530, 12754, 12980, 13209, 13440, 13673, 13909,
    14146, 14387, 14629, 14874, 15122, 15371, 15623, 15878, 16135, 16394, 16656, 16920, 17187, 17456, 17727, 18001,
    18277, 18556, 18837, 19121, 19407, 19696, 19987, 20281, 20577, 20876, 21177, 21481, 21787, 22096, 22407, 22721,
    23038, 23357, 23678, 24002, 24329, 24658, 24990, 25325, 25662, 26001, 26344, 26688, 27036, 27386, 27739, 28094,
    28452, 28813, 29176, 29542, 29911, 30282, 30656, 31033, 31412, 31794, 32179, 32567, 32957, 33350, 33745, 34143,
    34544, 34948, 35355, 35764, 36176, 36591, 37008, 37429, 37852, 38278, 38706, 39138, 39572, 40009, 40449, 40891,
    41337, 41785, 42236, 42690, 43147, 43606, 44069, 44534, 45002, 45473, 45947, 46423, 46903, 47385, 47871, 48359,
    48850, 49344, 49841, 50341, 50844, 51349, 51858, 52369, 52884, 53401, 53921, 54445, 54971, 55500, 56032, 56567,
    57105, 57646, 58190, 58737, 59287, 59840, 60396, 60955, 61517, 62082, 62650, 63221, 63795, 64372, 64952, 65535,
];

/// the WCAG 2 relative luminance of 0xRRGGBB (alpha ignored): 0 black .. 1 white
pub fn luminance(rgb: u32) -> f32 {
    let ch = |sh: u32| SRGB_LIN[((rgb >> sh) & 255) as usize] as f32 / 65535.0;
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
}

/// the WCAG 2 contrast ratio of two colours (alpha ignored): 1 (equal) .. 21 (black on
/// white); text needs 4.5 (large text 3), graphical objects and controls 3
pub fn contrast_ratio(a: u32, b: u32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

// the brightness of 0xRRGGBB (0..255)
fn luma(rgb: u32) -> u32 {
    ((rgb >> 16 & 255) * 299 + (rgb >> 8 & 255) * 587 + (rgb & 255) * 114) / 1000
}

// the colours a theme sets for a mode over the tokens
fn look_tokens(t: &mut [u32; T_COUNT], th: &Theme, mode: usize) {
    for (v, c) in t.iter_mut().zip(th.colors[mode].iter()) {
        if let Some(c) = c {
            *v = *c;
        }
    }
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

    /// reduced motion on (Some(true): no tweens, no fades), off (Some(false)) or as the
    /// system says (None); like --anim, a forced value outlasts the system's changes
    pub fn set_reduced_motion(&mut self, on: Option<bool>) {
        let before = self.anim_off;
        match on {
            Some(v) => {
                self.anim_forced = true;
                self.anim_off = v;
            }
            None => {
                self.anim_forced = false;
                self.anim_off = self.anim_sys;
            }
        }
        if before != self.anim_off {
            self.repass = true;
        }
    }

    /// the text scale in percent (100 = the style's sizes)
    pub fn text_scale(&self) -> u32 {
        self.text_scale
    }

    /// (caches of laid-out text) the text scale and the changes of theme and style in one
    /// key: the text scale (50..300) in the low bits, look.serial above
    pub(crate) fn layout_scale(&self) -> u32 {
        self.text_scale | self.look.serial << 16
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
        self.anim_sys = s.reduce_motion;
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
            let mut t = contrast_tokens(&c);
            if let Some(th) = &self.look.theme {
                if let Some(a) = th.accent[THEME_CONTRAST] {
                    for tok in [T_ACCENT, T_ACCENT_HOVER, T_ACCENT_PRESSED] {
                        t[(tok / 4) as usize] = a | 0xFF00_0000;
                    }
                }
                look_tokens(&mut t, th, THEME_CONTRAST);
            }
            t
        } else {
            self.theme_dark = self.theme_base_dark;
            let st = self.style;
            let mut t = st.tokens(self.theme_dark);
            let mode = if self.theme_dark { THEME_DARK } else { THEME_LIGHT };
            // the theme's accent wins over the app's and the system's
            let acc = self.look.theme.as_ref().and_then(|th| th.accent[mode]).unwrap_or(self.accent);
            st.accent(&mut t, self.theme_dark, acc); // the style derives the accent tokens
            if let Some(th) = &self.look.theme {
                look_tokens(&mut t, th, mode);
            }
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
        let (fx, font) = if self.look.text {
            self.look_ts(ts)
        } else {
            (self.style.ts_size_fx()[ts] as u32, self.style.ts_font()[ts] as u32)
        };
        let size = fx.wrapping_mul(self.wp.rt.dpi as u32) / 96;
        let size = if self.text_scale == 100 { size } else { size * self.text_scale / 100 };
        self.text.set_font(font, size as i32);
    }

    /// ui_ts_line(ts): line height of a text style in px
    pub fn ts_line(&self, ts: u32) -> i32 {
        let ts = if (ts & 7) < TS_COUNT { ts & 7 } else { TS_BODY } as usize;
        let dip = if self.look.text { self.look_line(ts) } else { self.style.ts_line()[ts] as i32 };
        self.dp(if self.text_scale == 100 { dip } else { (dip * self.text_scale as i32 + 50) / 100 })
    }

    // (theme) size (24.8 DIP) and font id of a text style: the theme's size and weight, the
    // family's face of that weight
    fn look_ts(&self, ts: usize) -> (u32, u32) {
        let st = self.style;
        let t = self.look.theme.as_ref().map_or(TextStyle::default(), |th| th.text[ts]);
        let fx = t.size_fx.map_or(st.ts_size_fx()[ts] as u32, u32::from);
        let face = match t.weight {
            Some(w) if w >= 650 => FONT_BOLD,
            Some(w) if w >= 500 => FONT_SEMI,
            Some(_) => FONT_REG,
            None => st.ts_font()[ts] as u32,
        };
        (fx, if face < 3 { self.look.fonts[face as usize] } else { face })
    }

    // (theme) the line height of a text style in DIP: the theme's, else the style's (grown
    // with a size of the theme)
    fn look_line(&self, ts: usize) -> i32 {
        let st = self.style;
        let base = st.ts_line()[ts] as i32;
        let t = self.look.theme.as_ref().map_or(TextStyle::default(), |th| th.text[ts]);
        match (t.line, t.size_fx) {
            (Some(l), _) => l as i32,
            (None, Some(s)) => {
                let ss = st.ts_size_fx()[ts] as i32;
                if ss > 0 { (base * s as i32 + ss / 2) / ss } else { base }
            }
            _ => base,
        }
    }

    // ------------------------------------------------------------------ app themes

    /// the app's theme over the style (theme_app.rs) from the next frame on: colours of light,
    /// dark and high contrast, the accent, corners, spacing, control heights, text styles and
    /// a font family.  A font that cannot be loaded is an error; the previous theme stays.
    pub fn set_theme(&mut self, th: &Theme) -> Result<(), ThemeError> {
        let fonts = self.theme_fonts(th)?;
        let l = &mut self.look;
        (l.rad_mode, l.rad) = match th.radius {
            None => (0, 0),
            Some(Radius::Dip(d)) => (1, d.min(64) as i32),
            Some(Radius::Percent(p)) => (2, p.min(400) as i32),
        };
        l.spacing = (th.spacing as i32).clamp(25, 400);
        l.ctrl_h = if th.control_height == 0 { 0 } else { (th.control_height as i32).clamp(12, 96) };
        l.text = th.has_text();
        l.fonts = fonts;
        l.theme = Some(Box::new(th.clone()));
        self.look_changed();
        Ok(())
    }

    /// no app theme: the style's own look again
    pub fn clear_theme(&mut self) {
        if self.look.theme.is_none() {
            return;
        }
        let l = &mut self.look;
        l.theme = None;
        (l.rad_mode, l.rad, l.spacing, l.ctrl_h, l.text) = (0, 0, 100, 0, false);
        l.fonts = [FONT_REG, FONT_SEMI, FONT_BOLD];
        self.look_changed();
    }

    /// the active app theme (None: the style's look)
    pub fn app_theme(&self) -> Option<&Theme> {
        self.look.theme.as_deref()
    }

    /// read a theme file (app://, config://, a path) and make it the app's theme; on an error
    /// the previous theme stays
    pub fn load_theme(&mut self, path: &str) -> Result<(), ThemeError> {
        let th = Theme::load(path)?;
        self.set_theme(&th)
    }

    // the font ids of a theme's family for FONT_REG / _SEMI / _BOLD (none: the style's)
    fn theme_fonts(&mut self, th: &Theme) -> Result<[u32; 3], ThemeError> {
        if th.family.is_empty() {
            return Ok([FONT_REG, FONT_SEMI, FONT_BOLD]);
        }
        let reg = self.theme_font(&th.family, 400)?;
        let face = |ui: &mut Ui, own: &str, other: &str, weight: u32| -> Result<u32, ThemeError> {
            if !own.is_empty() {
                ui.theme_font(own, weight)
            } else if !other.is_empty() {
                ui.theme_font(other, weight)
            } else if app::is_file(&th.family) {
                // a file without its bold faces: a synthetic bold
                let id = ui.text.style(reg, text::FS_SYN_BOLD);
                if id < 0 { Err(ThemeError::general(app::E_FONT, &th.family)) } else { Ok(id as u32) }
            } else {
                ui.theme_font(&th.family, weight)
            }
        };
        let semi = face(self, &th.family_semibold, &th.family_bold, 600)?;
        let bold = face(self, &th.family_bold, &th.family_semibold, 700)?;
        Ok([reg, semi, bold])
    }

    // one face: a font file (VFS path) or an installed family (ui.font_load)
    fn theme_font(&mut self, spec: &str, weight: u32) -> Result<u32, ThemeError> {
        let err = || ThemeError::general(app::E_FONT, spec);
        if !app::is_file(spec) {
            return self.font_load(&FontSel { family: String::from(spec), weight, italic: false, size: 11.0 }).ok_or_else(err);
        }
        if let Some(&(_, id)) = self.look.files.iter().find(|f| f.0 == spec) {
            return Ok(id);
        }
        let data = plat::read_file(spec).ok_or_else(err)?;
        let id = self.text.load_mem(&data, 0);
        if id < 0 {
            return Err(err());
        }
        self.look.files.push((String::from(spec), id as u32));
        Ok(id as u32)
    }

    // the theme or the style changed: tokens again, everything laid out and drawn again
    fn look_changed(&mut self) {
        self.look.serial = self.look.serial.wrapping_add(1);
        self.theme_build();
        self.theme_invalidate();
        self.repass = true;
    }

    /// (theme) a corner radius in px of a rounded rectangle w x h: corners that are not pills
    /// take the theme's radius (square ones stay square)
    pub(crate) fn look_radius(&self, r: i32, w: i32, h: i32) -> i32 {
        let half = w.min(h) >> 1;
        if r <= 0 || r >= half {
            return r;
        }
        let n = if self.look.rad_mode == 1 { self.dp(self.look.rad) } else { (r * self.look.rad + 50) / 100 };
        n.clamp(0, half)
    }

    /// (theme) DIP of box padding and gaps -> px, scaled by the theme's spacing
    pub(crate) fn dp_space(&self, dip: i32) -> i32 {
        let s = self.look.spacing;
        self.dp(if s == 100 { dip } else { (dip * s + 50) / 100 })
    }

    /// (theme) the size of a standard control with the theme's control height (flags ST_*;
    /// compact and large controls keep theirs); square: an icon-only control stays square
    pub(crate) fn ctrl_size(&self, w: i32, h: i32, flags: u32, square: bool) -> (i32, i32) {
        if self.look.ctrl_h == 0 || flags & (ST_LARGE | ST_COMPACT) != 0 {
            return (w, h);
        }
        let nh = self.dp_text(self.look.ctrl_h);
        (if square && w == h { nh } else { w }, nh)
    }

    // ------------------------------------------------------------------ the style at run time

    /// the styles set_style can switch to: the app names the style crates it links besides its
    /// own (build_rs.py links every style_NAME its sources name):
    /// `ui.styles_add(&[&style_classic::STYLE, &style_kde::STYLE])`
    pub fn styles_add(&mut self, styles: &[&'static dyn Style]) {
        for &s in styles {
            if !self.look.styles.iter().any(|x| x.name() == s.name()) {
                self.look.styles.push(s);
            }
        }
    }

    /// the names of the styles set_style knows (the app's own first, then as added)
    pub fn style_names(&self) -> Vec<&'static str> {
        self.look.styles.iter().map(|s| s.name()).collect()
    }

    /// the name of the style in use (Style::name)
    pub fn style_name(&self) -> &'static str {
        self.style.name()
    }

    /// switch to another linked style (styles_add) from the next frame on: its tokens, fonts,
    /// metrics and painters; the app theme stays on top -> false: no such style
    pub fn set_style(&mut self, name: &str) -> bool {
        let Some(&st) = self.look.styles.iter().find(|s| s.name() == name) else { return false };
        if self.style.name() == name {
            return true;
        }
        self.style = st;
        self.text.set_embedded(st.fonts(), st.font_names());
        for (font, raster) in st.font_raster().iter().enumerate() {
            self.text.set_font_raster(font as u32, *raster);
        }
        self.look_changed();
        true
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
