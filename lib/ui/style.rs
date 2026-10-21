// style -- the contract between the core and a style                (ASMLIB/inc/style.inc)
//
// The core holds every widget's logic (measuring, placing, input, state, semantics); how
// things look comes from exactly one style crate, chosen when the app is linked
// (build_rs.py --style S, the app passes `&style::STYLE` to run).  Size functions return
// the widget's size and may keep what they computed in Sp::s for the paint call; paint
// functions see the placed rectangle and the state and emit draw commands -- no layout,
// no input, no semantic records.
//
// The contract is split like the files of a style (ASMLIB/styles/<name>/*.s): Style (theme,
// type ramp, fonts) and one sub-trait per widget family -- StyleBasic (basic.s), StyleValue
// (value.s), StyleEdit (edit.s), StyleList (list.s), StylePopup (popup.s), StyleChrome
// (chrome.s), StyleMisc (misc.s).  R5: the default bodies (empty / zero) exist only while
// the widget families are being ported; a finished style overrides every function.

use alloc::rc::Rc;

use crate::consts::T_COUNT;
use crate::value::Track;
use crate::Ui;

/// the SP record: inputs of a widget's size / paint call (SP_* of style.inc)
#[derive(Clone, Copy, Default)]
pub struct Sp<'a> {
    /// placed rectangle (px; filled before the paint call)
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// 0 rest, 1 hover, 2 pressed, 3 disabled
    pub state: u32,
    /// EV_* of behave()
    pub ev: u32,
    /// ST_* / BOX_* / MIF_* / RG_* flags of the widget
    pub flags: u32,
    /// icon code point (0 = none)
    pub icon: u32,
    pub label: Option<&'a str>,
    pub value: i32,
    pub value2: i32,
    /// kept by the style from its size call to its paint call (SP_S0 .. SP_S3)
    pub s: [i32; 4],
    /// the 8-byte SP_VALUE variants: a second text (menu accelerator, info bar message)
    pub label2: Option<&'a str>,
    /// ... an f64 (angle degrees)
    pub f: f64,
    /// SP_LABEL variants: a thumbnail (pan), a TRACK record (value sliders: flags, n, stops)
    // ui-list: the thumbnail as the Rc draw_image takes, with its version (img_ver); the
    // TRACK record as value::Track (it may carry a custom drawing)
    pub img: Option<&'a Rc<gfx::Surface>>,
    pub img_ver: u32,
    pub track: Option<&'a Track>,
}

impl Sp<'_> {
    pub fn rect(&self) -> [i32; 4] {
        [self.x, self.y, self.w, self.h]
    }

    pub fn place(&mut self, r: [i32; 4]) {
        self.x = r[0];
        self.y = r[1];
        self.w = r[2];
        self.h = r[3];
    }
}

/// a style: tokens, type ramp, fonts and every widget family's paint functions
pub trait Style: StyleBasic + StyleValue + StyleEdit + StyleList + StylePopup + StyleChrome + StyleMisc + Sync {
    /// Ribbon height in DIP. Existing styles retain the original 110-DIP layout.
    fn ribbon_height(&self) -> i32 { 110 }
    /// Outer ribbon padding in DIP, independently styled from control content heights.
    fn ribbon_padding(&self) -> u32 { 8 }
    /// Optional document-style gallery face, size in 24.8 DIP and ink color.
    /// None preserves the ordinary UI type ramp in existing style previews.
    fn ribbon_preview(&self, _typography: u32) -> Option<(u32, i32, u32)> { None }
    /// Group sizing and overflow policy. Existing styles preserve compact flyouts.
    fn ribbon_metrics(&self) -> crate::RibbonMetrics { crate::RibbonMetrics::default() }
    /// Optional optical offset of a preview card in 24.8 DIP. Opts into the
    /// fractional panel painter; None keeps the original integer painter.
    fn preview_card_offset(&self) -> Option<[i32; 2]> { None }
    // ---- colours, type ramp, fonts (theme.s, fonts.s)
    /// the T_* tokens (light or dark; accent entries may be 0: accent() fills them)
    fn tokens(&self, dark: bool) -> [u32; T_COUNT];
    /// fill T_ACCENT, _HOVER, _PRESSED and T_ACCENT_TEXT from 0xRRGGBB (0 = the default)
    fn accent(&self, theme: &mut [u32; T_COUNT], dark: bool, rgb: u32);
    /// font px / line px / FONT_* of TS_CAPTION .. TS_DISPLAY (+ one spare) at 100 %
    fn ts_size(&self) -> [u8; 8];
    /// Font sizes in 24.8 DIP. The default keeps every existing integer type ramp exact.
    fn ts_size_fx(&self) -> [u16; 8] {
        self.ts_size().map(|size| u16::from(size) * 256)
    }
    fn ts_line(&self) -> [u8; 8];
    fn ts_font(&self) -> [u8; 8];
    /// Embedded SGF1 or TrueType/OpenType assets of FONT_REG, FONT_SEMI, FONT_BOLD, FONT_ICON.
    fn fonts(&self) -> [&'static [u8]; 4];
    /// Raster policy for the four embedded UI faces. Runtime/document fonts keep
    /// their own defaults. Existing styles preserve their pixel output.
    fn font_raster(&self) -> [text::FontRaster; 4] { [text::FontRaster::Legacy; 4] }
    /// per embedded font: family, style, full, typographic family, typographic style
    /// (sty_font_names)
    fn font_names(&self) -> [[&'static str; 5]; 4];
    /// Paint a semantic icon with the shared rasterizer; false uses the icon font.
    /// Coordinates are target pixels, size is 24.8 pixels; dpi permits fixed DIP details.
    /// Application icon hooks
    /// run first. Existing styles retain their original monochrome font rendering.
    #[allow(clippy::too_many_arguments)]
    fn paint_icon(&self, _g: &mut crate::Gfx, _cp: u32, _x: i32, _y: i32,
                  _size_fx: i32, _color: u32, _accent: u32, _dark: bool, _disabled: bool, _dpi: u32) -> bool {
        false
    }
    /// The painters of the data views (grids, trees, badges, meters, charts).  The default
    /// paints them from the theme tokens (stock.rs), so every style shows them; a style
    /// returns its own StyleData to give them its look.
    fn data(&self) -> &'static dyn StyleData {
        &crate::stock::Stock
    }
    /// The painters of the app shell parts (navigation pane, command bar, splitter,
    /// breadcrumb, segmented control, wizard steps, toasts, spinner, calendar).  Default:
    /// token-based (stock.rs).
    fn shell(&self) -> &'static dyn StyleShell {
        &crate::stock::Stock
    }
}

/// data views: grid header / rows, tree rows, badges, status dots, meters, charts  (data.rs,
/// chart.rs).  Every method has a token-based default (stock.rs); a style overrides what it
/// wants to look different.
pub trait StyleData {
    /// (header strip height, row height, cell padding) in DIP
    fn grid_metrics(&self, _ui: &Ui) -> (i32, i32, i32) {
        (32, 36, 12)
    }
    /// the header strip behind the header cells (rect = the strip)
    fn grid_header_bg(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::grid_header_bg(ui, sp)
    }
    /// header cell: label, ev, flags GH_*; s[0] = the label inset (px)
    fn grid_header(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::grid_header(ui, sp)
    }
    /// row background: ev, value 1 = selected, value2 = row index, flags GR_*
    fn grid_row(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::grid_row(ui, sp)
    }
    /// tree expander glyph: value 1 = expanded, ev
    fn tree_twisty(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::tree_twisty(ui, sp)
    }
    /// badge: flags BADGE_*, label; size -> (w, h)
    fn badge_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        crate::stock::badge_size(ui, sp)
    }
    fn badge(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::badge(ui, sp)
    }
    /// status dot: flags BADGE_*
    fn status_dot(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::status_dot(ui, sp)
    }
    /// meter (capacity bar): value, value2 maximum, flags BADGE_* of the fill
    fn meter(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::meter(ui, sp)
    }
    /// the categorical colours of charts (series 0 .. 7, 0xAARRGGBB)
    fn chart_palette(&self, ui: &Ui) -> [u32; 8] {
        crate::stock::chart_palette(ui)
    }
}

/// app shell parts: navigation pane and items, command bar, splitter grip, breadcrumb,
/// segmented control, wizard steps, toasts, spinner, indeterminate progress, calendar cells
/// (shell.rs, notify.rs, datepick.rs).  Token-based defaults in stock.rs.
pub trait StyleShell {
    /// the navigation pane's surface (rect = the pane), flags 1 = compact
    fn nav_pane(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::nav_pane(ui, sp)
    }
    /// navigation item: ev, value 1 = selected, icon, label, label2 = badge text, flags
    /// NAV_*; size -> (w, h)
    fn nav_item_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        crate::stock::nav_item_size(ui, sp)
    }
    fn nav_item(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::nav_item(ui, sp)
    }
    /// section header of the navigation pane: label, flags NAV_COMPACT (then a divider)
    fn nav_section_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        crate::stock::nav_section_size(ui, sp)
    }
    fn nav_section(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::nav_section(ui, sp)
    }
    /// the bar behind a command bar (rect = the bar)
    fn command_bar(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::command_bar(ui, sp)
    }
    /// splitter grip: ev, flags 1 = vertical split (a horizontal grip)
    fn split_grip(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::split_grip(ui, sp)
    }
    /// breadcrumb separator (rect = its place)
    fn breadcrumb_sep(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::breadcrumb_sep(ui, sp)
    }
    /// segmented control item: ev, value 1 = selected, label, flags SEG_FIRST / SEG_LAST
    fn segment(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::segment(ui, sp)
    }
    /// wizard step row: value step number (from 1), value2 WS_* state, label, ev
    fn wizard_step(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::wizard_step(ui, sp)
    }
    /// toast card: flags IB_* severity, label title, label2 message; s[0] = close button
    /// x, s[1] its size (set by the painter, hence &mut)
    fn toast(&self, ui: &mut Ui, sp: &mut Sp) {
        crate::stock::toast(ui, sp)
    }
    /// spinner: value phase 0 .. 1023 (one turn), rect = its square
    fn spinner(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::spinner(ui, sp)
    }
    /// indeterminate progress bar: value phase 0 .. 1023
    fn progress_indeterminate(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::progress_indeterminate(ui, sp)
    }
    /// day cell of a calendar: label, value 1 = selected, value2 1 = today, flags 1 = other
    /// month, ev
    fn calendar_day(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::calendar_day(ui, sp)
    }
    /// a dock group's surface (rect = tab strip and content; dock.rs), value 1 = the keyboard
    /// focus is in the group
    fn dock_group(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::dock_group(ui, sp)
    }
    /// the tab strip of a dock group (rect = the strip, SM_DOCK_HDR high), value as dock_group
    fn dock_header(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::dock_header(ui, sp)
    }
    /// a tab of a dock group: label, icon, ev, value 1 = the shown panel, value2 1 = the focus
    /// is in the group; size -> (w, h), the core takes w (a tab is as high as the strip)
    fn dock_tab_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        crate::stock::dock_tab_size(ui, sp)
    }
    fn dock_tab(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::dock_tab(ui, sp)
    }
    /// the rail of a collapsed dock side (rect), flags DOCK_LEFT / DOCK_RIGHT / DOCK_BOTTOM
    fn dock_rail(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::dock_rail(ui, sp)
    }
    /// a drop marker while a panel is dragged: value DOCK_MK_* (the part of the place it
    /// fills), ev EV_HOVER while the pointer is over it
    fn dock_marker(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::dock_marker(ui, sp)
    }
    /// where the dragged panel would go (rect), value 1 = a floating window there
    fn dock_preview(&self, ui: &mut Ui, sp: &Sp) {
        crate::stock::dock_preview(ui, sp)
    }
}

/// buttons, check boxes, radios, switches, sliders, progress, separators  (basic.s)
pub trait StyleBasic {
    /// focus visual around rectangle r (px) of a control with that corner radius (DIP)
    fn focus_ring(&self, _ui: &mut Ui, _r: [i32; 4], _radius_dip: i32) {}
    /// a drop target with a matching drag over it (rectangle r, px; drop_target): by default
    /// a 2 DIP accent frame over a light accent tint
    fn drop_highlight(&self, ui: &mut Ui, r: [i32; 4]) {
        let c = ui.color(crate::consts::T_ACCENT);
        let tint = (c & 0x00FF_FFFF) | 0x2E00_0000;
        let (t, rad) = (ui.dp(2), ui.dp(4));
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, tint, c, c);
    }
    /// buttons (ST_ACCENT, ST_SUBTLE, ST_CHECKED, ST_LARGE, ST_COMPACT, ST_ICON_ONLY)
    fn button_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn button(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// value 0 / 1 / 2 (indeterminate)
    fn checkbox_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn checkbox(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// value 1 = selected
    fn radio_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn radio(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// value 0 / 1, value2 knob position 0 .. 256 (animated)
    fn switch_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn switch(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// size: value 1 = vertical; must set s[0] = thumb px along the axis; paint: value thumb
    /// offset, value2 track length (px)
    fn slider_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn slider(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// value, value2 maximum
    fn progress_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn progress(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// thickness incl. margins; paint: value 1 = horizontal line (in a vertical box)
    fn separator_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> i32 {
        0
    }
    fn separator(&self, _ui: &mut Ui, _sp: &Sp) {}
}

/// value sliders (gradient tracks), angle, pan  (value.s)
pub trait StyleValue {
    /// flags RG_*, value t 0..65536 along the travel, track (gradient) or None; state held = 2
    fn value_slider_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn value_slider(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// thumb length px (travel = length - thumb)
    fn value_slider_thumb(&self, _ui: &mut Ui) -> i32 {
        0
    }
    /// f degrees, s[0] 1 = focus visual
    fn angle_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn angle(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// img thumbnail or None; paint: value, value2 = the point (px)
    fn pan_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn pan(&self, _ui: &mut Ui, _sp: &Sp) {}
}

/// text boxes  (edit.s)
pub trait StyleEdit {
    /// Select the single-line input face and return its line height and baseline in px.
    /// Default preserves the body font; icon inputs may use a compact search-field face.
    fn textbox_font(&self, ui: &mut Ui, _sp: &Sp) -> (i32, i32) {
        (ui.ts_line(crate::consts::TS_BODY), ui.ts_base(crate::consts::TS_BODY))
    }
    /// value natural width (DIP), flags 1 = multi-line (value2 line height px); must set
    /// s[0], s[1], s[2] = left, right, top text inset (px)
    fn textbox_metrics(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    /// ev (hover), value 1 = editing, value2 1 = disabled; the core draws text, selection, caret
    fn textbox_frame(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// value view h, value2 content h, s[0] scroll y, s[1] text top (px)
    fn textbox_vbar(&self, _ui: &mut Ui, _sp: &Sp) {}
}

/// focus rectangles, scroll bars, list rows, tabs, expanders  (list.s)
pub trait StyleList {
    /// focus visual drawn ON a rectangle (rows, tabs, headers)
    fn focus_rect(&self, _ui: &mut Ui, _x: i32, _y: i32, _w: i32, _h: i32) {}
    /// hover area, track margin at both ends, shortest thumb (px)
    fn scrollbar_thickness(&self, _ui: &mut Ui) -> i32 {
        0
    }
    fn scrollbar_margin(&self, _ui: &mut Ui) -> i32 {
        0
    }
    fn scrollbar_min_thumb(&self, _ui: &mut Ui) -> i32 {
        0
    }
    /// flags 1 = vertical, value expansion 0..256, s[0] track start, s[1] thumb length,
    /// value2 thumb offset (px)
    fn scrollbar(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// reorder marker centred on y across x .. + w
    fn list_drop_line(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// ev, value 1 = selected, value2 1 = disabled, flags 1 = dragged row, 2 = focus visual
    fn list_row(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// sets s[0] label padding, s[1] height, s[2] pill width
    fn tab_metrics(&self, _ui: &mut Ui, _sp: &mut Sp) {}
    /// Select the font used to measure or paint a tab; default preserves the body face.
    fn tab_font(&self, ui: &mut Ui, _selected: bool) {
        ui.ts_font(crate::consts::TS_BODY);
    }
    /// tab: ev, value selected, value2 disabled, label; pill: strip rect, value pill x
    fn tab(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn tab_pill(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// header: ev, value open, value2 disabled, label; body: the content card's rectangle
    fn expander_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn expander_header(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn expander_body(&self, _ui: &mut Ui, _sp: &Sp) {}
}

/// flyouts, menus, menu bars, drop-downs, split buttons, combo boxes, tooltips  (popup.s)
pub trait StylePopup {
    /// px
    fn flyout_pad(&self, _ui: &mut Ui) -> i32 {
        0
    }
    /// BOX_* | PAD_* flags of the content
    fn flyout_box(&self, _ui: &mut Ui) -> u32 {
        0
    }
    /// shadow + surface of a flyout / menu (rect = the flyout)
    fn flyout_frame(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// row height px
    fn menu_item_h(&self, _ui: &mut Ui) -> i32 {
        0
    }
    /// flags MIF_*, label, label2 accelerator text, icon, ev; size must set s[1] =
    /// accelerator width
    fn menu_item_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn menu_item(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn menu_sep_size(&self, _ui: &mut Ui) -> i32 {
        0
    }
    fn menu_sep(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// flags 1 open, 2 current item, value mnemonic byte index or -1, label, ev
    fn menubar_item_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn menubar_item(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// flags ST_*, icon, label, value open, value2 disabled
    fn dropdown_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn dropdown(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// size sets s[0] / s[1] = main / chevron part width; paint: ev main, s[2] chevron
    /// events, value open, value2 1 main / 2 chevron disabled
    fn split_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn split(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// size: value widest text px; paint: label the selected text, ev, value2 disabled
    fn combo_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn combo(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// size sets s[0] text width, s[1] / s[2] gap below / above the pointer; paint: label
    fn tooltip_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn tooltip(&self, _ui: &mut Ui, _sp: &Sp) {}
}

/// layout metrics, dialogs, info bars, status / title bars, caption buttons, window frames
/// (chrome.s)
pub trait StyleChrome {
    /// Background for navigation placed on a branded accent surface.
    fn navigation_surface(&self, ui: &Ui) -> u32 {
        ui.color(crate::consts::T_ACCENT)
    }
    /// layout value SM_* (sizes, gaps, box flags, text styles)
    fn metric(&self, _id: u32) -> i32 {
        0
    }
    fn dialog_smoke(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// dialog surface (value footer h)
    fn dialog_frame(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// size: flags severity, label title, label2 message, sets s[0] / s[1] title / message
    /// width; paint sets s[2] / s[3] = close button x / size (hence &mut); close: ev
    fn infobar_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn infobar(&self, _ui: &mut Ui, _sp: &mut Sp) {}
    fn infobar_close(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn statusbar(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// ev, flags HB_* (HB_CLOSE), icon, value 1 = window active
    fn caption_button(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// background of a library title bar: rect = the bar, value 1 = active
    fn window_titlebar(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// value thickness px
    fn window_border(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn window_dialog_fill(&self, _ui: &mut Ui, _sp: &Sp) {}
}

/// swatches, colour picker parts, box decorations, window background  (misc.s)
pub trait StyleMisc {
    /// value colour, value2 1 = selected, ev
    fn swatch_size(&self, _ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        (0, 0)
    }
    fn swatch(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn color_preview(&self, _ui: &mut Ui, _argb: u32, _r: [i32; 4]) {}
    /// marker at 24.8 px
    fn color_marker(&self, _ui: &mut Ui, _x_fx: i32, _y_fx: i32) {}
    /// rect = the wheel's square, s[0] radius px, value hue (tenths of a degree), value2
    /// saturation 0..1000, s[1] 1 = focus visual
    fn color_wheel(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// BOX_BG / BOX_CARD / BOX_LAYER decoration (sp: rectangle, flags)
    fn box_deco(&self, _ui: &mut Ui, _sp: &Sp) {}
    /// the background of a window's whole surface (px)
    fn window_bg(&self, _ui: &mut Ui, _w: i32, _h: i32) {}
}
