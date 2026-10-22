// themedemo -- app themes at run time and the switch between linked styles
//   (ui/theme.rs, ui/theme_app.rs; docs/STYLES.md "App themes", GUIDE §34)
//
//   python build_rs.py app themedemo
//   build/win/themedemo.exe --shot t.png [--theme-file apps/themedemo/assets/themes/ocean.theme]
//   build/win/themedemo.exe --script apps/themedemo/tests/themes.uis
//
// Left the page "Darstellung": the style (the style crates this app names -- style_winui,
// style_classic, style_kde, style_macos -- are linked into it by build_rs.py; ui.set_style),
// light / dark / system, high contrast, the theme ("Ohne Theme" = the style's own look, the
// two sample themes of app://themes, "Eigenes" = the edited one), a few tokens edited live
// (accent, background, corner radius, spacing, control height, text size, a system font),
// "Speichern" / "Gespeichertes laden" (the edited theme as a theme file in
// config://themes/eigenes.theme; headless runs use mem://themedemo/eigenes.theme, so tests
// never touch real folders) and "Laden" (a theme file by its path; a broken one shows its
// error in an info bar and the theme stays).  Right a preview of the common widgets.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::theme::{Radius, Theme, THEME_DARK, THEME_LIGHT};
use ui::*;

const DESC: AppDesc = AppDesc { title: "Themes", flags: AF_RESIZABLE, w: 980, h: 760, min_w: 640, min_h: 480, ..AppDesc::NONE };

const THEMES: [&str; 2] = ["app://themes/ocean.theme", "app://themes/paper.theme"];
const CHOICES: [&str; 4] = ["Ohne Theme", "Ozean", "Papier", "Eigenes"];
const CUSTOM: usize = 3;
// the editor's colour presets (0xRRGGBB): accents; backgrounds light, dark
const ACCENTS: [u32; 5] = [0x0F7B8A, 0xB5482A, 0x6B4FBB, 0x2E7D32, 0xC2185B];
const BGS_LIGHT: [u32; 4] = [0xF3F3F3, 0xEEF3F8, 0xF6F1E7, 0xEEF6EE];
const BGS_DARK: [u32; 4] = [0x202020, 0x172029, 0x26211B, 0x1B241C];

// ids: composite widgets (segmented control, tabs) use the ids after their own
const ID_PAGE: u32 = 90;
const ID_STYLE: u32 = 100;
const ID_MODE: u32 = 110; // + item
const ID_CONTRAST: u32 = 120;
const ID_THEME: u32 = 130; // + choice
const ID_ACCENT: u32 = 140; // + preset
const ID_BG: u32 = 150; // + preset
const ID_RADIUS: u32 = 160;
const ID_SPACING: u32 = 161;
const ID_HEIGHT: u32 = 162;
const ID_SIZE: u32 = 163;
const ID_FONT_ON: u32 = 164;
const ID_FONT: u32 = 170;
const ID_SAVE: u32 = 180;
const ID_RELOAD: u32 = 181;
const ID_PATH: u32 = 182;
const ID_LOAD: u32 = 183;
const ID_ERR: u32 = 184;
const ID_P_BTN: u32 = 200;
const ID_P_ACC: u32 = 201;
const ID_P_DIS: u32 = 202;
const ID_P_CB: u32 = 203;
const ID_P_R1: u32 = 204;
const ID_P_R2: u32 = 205;
const ID_P_SW: u32 = 206;
const ID_P_NAME: u32 = 207;
const ID_P_COMBO: u32 = 208;
const ID_P_SLIDER: u32 = 209;
const ID_P_PROGRESS: u32 = 210;
const ID_P_DROP: u32 = 211;
const ID_P_TABS: u32 = 220; // + tab

const RANGE_RADIUS: Range = Range::new(0, 16, 1, 0);
const RANGE_SPACING: Range = Range::new(50, 200, 10, 0);
const RANGE_HEIGHT: Range = Range::new(0, 64, 2, 0);
const RANGE_SIZE: Range = Range::new(10, 24, 1, 0);
const RANGE_PCT: Range = Range::new(0, 100, 1, 0);

struct Demo {
    /// CHOICES index of the active theme
    sel: i32,
    /// the theme "Eigenes" (the editor's)
    edit: Theme,
    mode: i32,
    style: i32,
    font_on: i32,
    font: FontSel,
    path: String,
    msg: String,
    err: String,
    // the preview's widgets
    cb: i32,
    radio: i32,
    sw: i32,
    slider: i32,
    name: String,
    combo: i32,
    tab: i32,
}

impl Default for Demo {
    fn default() -> Demo {
        Demo {
            sel: 0,
            edit: Theme { name: String::from("Eigenes"), ..Theme::default() },
            mode: 0,
            style: 0,
            font_on: 0,
            font: FontSel::new("sans-serif", 11.0),
            path: String::new(),
            msg: String::new(),
            err: String::new(),
            cb: 1,
            radio: 0,
            sw: 1,
            slider: 60,
            name: String::new(),
            combo: 0,
            tab: 0,
        }
    }
}

// where "Speichern" puts the edited theme
fn saved_path(ui: &Ui) -> &'static str {
    if ui.headless() { "mem://themedemo/eigenes.theme" } else { "config://themes/eigenes.theme" }
}

impl Demo {
    // make a choice of CHOICES the active theme
    fn choose(&mut self, ui: &mut Ui, k: usize) {
        let res = match k {
            0 => {
                ui.clear_theme();
                Ok(())
            }
            CUSTOM => ui.set_theme(&self.edit),
            _ => ui.load_theme(THEMES[k - 1]),
        };
        match res {
            Ok(()) => {
                self.sel = k as i32;
                self.err.clear();
            }
            Err(e) => self.err = format!("{}", e),
        }
    }

    // the editor changes the theme: it starts from the active one, then "Eigenes" is shown
    fn edited(&mut self, ui: &mut Ui, f: impl FnOnce(&mut Theme)) {
        if self.sel as usize != CUSTOM {
            self.edit = ui.app_theme().cloned().unwrap_or_default();
            self.edit.name = String::from("Eigenes");
        }
        f(&mut self.edit);
        self.choose(ui, CUSTOM);
    }

    // the left column: style, mode, theme, editor, files
    fn page(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, "Darstellung");
        // ---- style: the linked style crates
        let names: Vec<&str> = ui.style_names();
        self.style = names.iter().position(|n| *n == ui.style_name()).unwrap_or(0) as i32;
        let mut st = self.style;
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(150, 0);
        ui.text(TS_BODY, "Stil");
        if ui.combobox(ID_STYLE, &mut st, &names) & EV_ACT != 0 && st != self.style {
            if let Some(n) = names.get(st as usize) {
                ui.set_style(n);
                self.msg = format!("Stil: {}", n);
            }
        }
        ui.box_end();
        // ---- light / dark / system, high contrast (scripts change them too)
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(150, 0);
        ui.text(TS_BODY, "Modus");
        if self.mode != 2 {
            self.mode = ui.theme_dark() as i32;
        }
        let mut m = self.mode;
        if ui.segmented(ID_MODE, &mut m, &["Hell", "Dunkel", "System"]) & EV_ACT != 0 && m != self.mode {
            self.mode = m;
            ui.theme_set(m as u32);
        }
        ui.box_end();
        let mut hc = ui.high_contrast() as i32;
        if ui.switch(ID_CONTRAST, "Hoher Kontrast", &mut hc) & EV_ACT != 0 {
            ui.set_contrast(Some(hc != 0));
        }
        ui.separator();
        // ---- the theme (also one of --theme-file or a script)
        self.sel = match ui.app_theme().map(|t| t.name.as_str()) {
            None => 0,
            Some("Ozean") => 1,
            Some("Papier") => 2,
            Some(_) => CUSTOM as i32,
        };
        ui.text(TS_BODY_STRONG, "Theme");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        let mut sel = self.sel;
        for (k, c) in CHOICES.iter().enumerate() {
            if ui.radio(ID_THEME + k as u32, c, &mut sel, k as i32) & EV_ACT != 0 {
                self.choose(ui, k);
            }
        }
        ui.box_end();
        // ---- the editor (values of the active theme; a change makes it "Eigenes")
        let th = ui.app_theme().cloned().unwrap_or_default();
        let dark = ui.theme_dark();
        let mode = if dark { THEME_DARK } else { THEME_LIGHT };
        ui.text(TS_BODY_STRONG, "Bearbeiten");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(150, 0);
        ui.text(TS_BODY, "Akzentfarbe");
        for (i, &c) in ACCENTS.iter().enumerate() {
            let on = th.accent[mode] == Some(c);
            if ui.swatch(ID_ACCENT + i as u32, 0xFF00_0000 | c, on) & EV_ACT != 0 {
                self.edited(ui, |t| {
                    t.accent[THEME_LIGHT] = Some(c);
                    t.accent[THEME_DARK] = Some(c);
                });
            }
        }
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(150, 0);
        ui.text(TS_BODY, "Hintergrund");
        let bgs = if dark { &BGS_DARK } else { &BGS_LIGHT };
        for (i, &c) in bgs.iter().enumerate() {
            let on = th.colors[mode][(T_BG / 4) as usize] == Some(0xFF00_0000 | c);
            if ui.swatch(ID_BG + i as u32, 0xFF00_0000 | c, on) & EV_ACT != 0 {
                self.edited(ui, |t| t.colors[mode][(T_BG / 4) as usize] = Some(0xFF00_0000 | c));
            }
        }
        ui.box_end();
        let mut rad = match th.radius {
            Some(Radius::Dip(d)) => d as i32,
            _ => 4,
        };
        if self.row_slider(ui, ID_RADIUS, &format!("Eckenradius: {} DIP", rad), &mut rad, &RANGE_RADIUS) {
            self.edited(ui, |t| t.radius = Some(Radius::Dip(rad as u16)));
        }
        let mut sp = th.spacing as i32;
        if self.row_slider(ui, ID_SPACING, &format!("Abstände: {} %", sp), &mut sp, &RANGE_SPACING) {
            self.edited(ui, |t| t.spacing = sp as u16);
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(150, 0);
        ui.text(TS_BODY, "Höhe (0 = Stil)");
        let mut h = th.control_height as i32;
        if ui.number_box(ID_HEIGHT, &mut h, &RANGE_HEIGHT) & EV_ACT != 0 && h != th.control_height as i32 {
            self.edited(ui, |t| t.control_height = h as u16);
        }
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(150, 0);
        ui.text(TS_BODY, "Textgröße");
        let ts = th.text[TS_BODY as usize].size_fx.map_or(14, |s| (s as i32 + 128) >> 8);
        let mut size = ts;
        if ui.number_box(ID_SIZE, &mut size, &RANGE_SIZE) & EV_ACT != 0 && size != ts {
            self.edited(ui, |t| {
                for k in [TS_BODY, TS_BODY_STRONG] {
                    t.text[k as usize].size_fx = Some((size << 8) as u16);
                    t.text[k as usize].line = None;
                }
            });
        }
        ui.box_end();
        // a system font for the theme (scanned when switched on)
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        let mut fo = self.font_on;
        if ui.switch(ID_FONT_ON, "Schrift des Systems", &mut fo) & EV_ACT != 0 {
            self.font_on = fo;
            let fam = if fo != 0 { self.font.family.clone() } else { String::new() };
            self.edited(ui, |t| t.family = fam);
        }
        if self.font_on != 0 && ui.font_picker(ID_FONT, &mut self.font) & EV_ACT != 0 {
            let fam = self.font.family.clone();
            self.edited(ui, |t| t.family = fam);
        }
        ui.box_end();
        // ---- files
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        if ui.button(ID_SAVE, "Speichern") & EV_ACT != 0 {
            let p = saved_path(ui);
            self.msg = if self.edit.save(p) { format!("Gespeichert: {}", p) } else { format!("Nicht gespeichert: {}", p) };
        }
        if ui.button(ID_RELOAD, "Gespeichertes laden") & EV_ACT != 0 {
            match Theme::load(saved_path(ui)) {
                Ok(t) => {
                    self.edit = t;
                    self.choose(ui, CUSTOM);
                }
                Err(e) => self.err = format!("{}", e),
            }
        }
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(-1, 0);
        ui.textbox(ID_PATH, Some("Pfad einer Theme-Datei"), &mut self.path, 260);
        if ui.button(ID_LOAD, "Laden") & EV_ACT != 0 {
            match ui.load_theme(&self.path) {
                Ok(()) => {
                    self.edit = ui.app_theme().cloned().unwrap_or_default();
                    self.sel = CUSTOM as i32;
                    self.err.clear();
                    self.msg = format!("Geladen: {}", self.edit.name);
                }
                Err(e) => self.err = format!("{}", e),
            }
        }
        ui.box_end();
        if !self.err.is_empty() {
            // the error in full below the bar (it names the line of the file)
            let err = self.err.clone();
            if ui.infobar(ID_ERR, IB_ERROR, "Theme nicht übernommen", "") & EV_ACT != 0 {
                self.err.clear();
            }
            ui.text(TS_BODY | TX_WRAP, &err);
        }
        let status = match ui.app_theme() {
            Some(t) => format!("Theme: {}", t.name),
            None => String::from("Kein Theme: der Stil, wie er ist"),
        };
        ui.text(TS_CAPTION | TX_SECONDARY, &status);
        if !self.msg.is_empty() {
            ui.text(TS_CAPTION | TX_SECONDARY, &self.msg);
        }
    }

    // a labelled slider row -> the value changed
    fn row_slider(&mut self, ui: &mut Ui, id: u32, label: &str, v: &mut i32, rg: &Range) -> bool {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(150, 0);
        ui.text(TS_BODY, label);
        let old = *v;
        ui.next_size(-1, 0);
        ui.slider(id, v, rg);
        ui.box_end();
        *v != old
    }

    // the right column: the common widgets in the theme
    fn preview(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, "Vorschau");
        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 12);
        ui.text(TS_TITLE, "Titel");
        ui.text(TS_BODY | TX_WRAP, "Ein Absatz Fließtext: Farben, Ecken, Abstände und Schrift kommen aus dem Theme über dem Stil.");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.button(ID_P_BTN, "Schaltfläche");
        ui.accent_button(ID_P_ACC, "Akzent");
        ui.next_flags(ST_DISABLED);
        ui.button(ID_P_DIS, "Deaktiviert");
        ui.dropdown_button(ID_P_DROP, "Menü");
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        ui.checkbox(ID_P_CB, "Option", &mut self.cb);
        ui.radio(ID_P_R1, "Eins", &mut self.radio, 0);
        ui.radio(ID_P_R2, "Zwei", &mut self.radio, 1);
        ui.switch(ID_P_SW, "Schalter", &mut self.sw);
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(200, 0);
        ui.textbox(ID_P_NAME, Some("Name"), &mut self.name, 64);
        ui.next_size(160, 0);
        ui.combobox(ID_P_COMBO, &mut self.combo, &["Erste Wahl", "Zweite Wahl", "Dritte Wahl"]);
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        ui.slider(ID_P_SLIDER, &mut self.slider, &RANGE_PCT);
        ui.progress(ID_P_PROGRESS, self.slider, 100);
        ui.box_end();
        ui.tabs(ID_P_TABS, &mut self.tab, &["Allgemein", "Ansicht", "Erweitert"]);
        ui.text(TS_CAPTION | TX_SECONDARY, "Beschriftung in der kleinsten Textgröße");
        ui.box_end();
    }
}

impl App for Demo {
    fn init(&mut self, ui: &mut Ui) -> bool {
        // the style crates named here are linked into the app (build_rs.py): set_style can
        // switch to each of them
        ui.styles_add(&[&style_winui::STYLE, &style_classic::STYLE, &style_kde::STYLE, &style_macos::STYLE]);
        self.mode = if ui.theme_dark() { 1 } else { 0 };
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.scroll_begin(ID_PAGE, SC_V);
        ui.box_begin(0, BOX_H | PAD_24, 24);
        ui.next_size(400, 0);
        ui.box_begin(0, BOX_V, 12);
        self.page(ui);
        ui.box_end();
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 12);
        self.preview(ui);
        ui.box_end();
        ui.box_end();
        ui.scroll_end();
    }
}

ui::main!(ui::run(&DESC, &mut Demo::default(), &style::STYLE));
