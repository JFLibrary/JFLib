// "Werte & Farben": decimal slider and number box on one value, property rows, the angle dial, the
// pan control, colour tracks (H S V R G B A), a swatch with a menu, the colour wheel with its value
// track, the colour picker with alpha and fields (apps/gallery/sec_list.rs: gal_values, gal_wheel).

use ui::*;

const ID_SF: u32 = 6000;
const ID_NBF: u32 = 6001;
const ID_PROP: u32 = 6002;
const ID_PROPF: u32 = 6003;
const ID_ANG: u32 = 6004;
const ID_PAN: u32 = 6005;
const ID_H: u32 = 6010; // tracks H S V R G B A: .. 6016
const ID_VT: u32 = 6017;
const ID_SW: u32 = 6018;
const ID_WHEEL: u32 = 6020;
const ID_WHEEL_V: u32 = 6021;
const ID_PICKER: u32 = 6030;
const ID_COPY: u32 = 6040;

const RF_VAL: RangeF = RangeF::new(0.0, 100.0, 0.5, 1, 0);
const RF_PROP: RangeF = RangeF::new(0.0, 10.0, 0.0, 2, 0);
const RANGE_PROP: Range = Range::new(0, 100, 1, 0);
const RANGE_HUE: Range = Range::new(0, 359, 1, 0);
const RANGE_PCT: Range = Range::new(0, 100, 1, 0);
const RANGE_PCT_V: Range = Range::new(0, 100, 1, RG_VERT);
const RANGE_BYTE: Range = Range::new(0, 255, 1, 0);
const RANGE_WV: Range = Range::new(0, 1000, 10, RG_VERT);
const TK_H: [u32; 7] = [0xFFFF0000, 0xFFFFFF00, 0xFF00FF00, 0xFF00FFFF, 0xFF0000FF, 0xFFFF00FF, 0xFFFF0000];
static M_SWATCH: [Mi; 1] = [Mi::Item(ID_COPY, "&Farbe merken", IC_COPY)];

pub struct Colors {
    val_f: f64,
    prop_i: i32,
    prop_f: f64,
    ang: f64,
    pan_x: f64,
    pan_y: f64,
    /// h, s, v, r, g, b, a
    trk: [i32; 7],
    tk: [Track; 7],
    argb: u32,
    whsv: [u32; 3],
    pick: u32,
    tk_wv: Track,
    kept: u32,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            val_f: 12.5,
            prop_i: 25,
            prop_f: 1.5,
            ang: 45.0,
            pan_x: 0.0,
            pan_y: 0.0,
            trk: [210, 80, 90, 46, 126, 230, 200],
            tk: [
                Track::new(0, &TK_H),
                Track::new(0, &[0, 0]),
                Track::new(0, &[0, 0]),
                Track::new(0, &[0, 0]),
                Track::new(0, &[0, 0]),
                Track::new(0, &[0, 0]),
                Track::new(TK_CHECKER, &[0, 0]),
            ],
            argb: 0,
            whsv: [1200, 700, 850],
            pick: 0xC0E0_4040,
            tk_wv: Track::new(0, &[0xFF00_0000, 0xFF00_FF00]),
            kept: 0,
        }
    }
}

impl Colors {
    pub fn page(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY_STRONG, "Dezimalwerte");
        ui.next_size(SIZE_FILL, 40);
        ui.slider_f(ID_SF, &mut self.val_f, &RF_VAL);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.next_size(SIZE_FILL, 44);
        ui.number_box_f(ID_NBF, &mut self.val_f, &RF_VAL);
        let t = ui.fmt_f64(self.val_f, 2);
        ui.next_size(64, 0);
        ui.text(TS_BODY | TX_RIGHT | TX_SECONDARY, &t);
        ui.box_end();

        ui.text(TS_BODY_STRONG, "Eigenschaften");
        ui.prop_slider(ID_PROP, "Radius", &mut self.prop_i, &RANGE_PROP, 25);
        ui.prop_slider_f(ID_PROPF, "Härte", &mut self.prop_f, &RF_PROP, Some(1.5));

        ui.text(TS_BODY_STRONG, "Winkel und Verschieben");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        ui.angle(ID_ANG, &mut self.ang, 0);
        let t = ui.fmt_f64(self.ang, 2);
        ui.next_size(56, 0);
        ui.text(TS_BODY | TX_RIGHT, &t);
        ui.pan(ID_PAN, &mut self.pan_x, &mut self.pan_y, None, 0, 0);
        ui.box_end();

        ui.text(TS_BODY_STRONG, "Farbspuren");
        self.tracks(ui);

        ui.text(TS_BODY_STRONG, "Farbrad");
        ui.box_begin(0, BOX_H | BOX_XSTART, 16);
        self.tk_wv.stops[1] = hsv_to_rgb(self.whsv[0], self.whsv[1], 1000);
        ui.color_wheel(ID_WHEEL, &mut self.whsv, CW_HSV);
        ui.next_size(40, 160);
        let mut v = self.whsv[2] as i32;
        let tk = self.tk_wv.clone();
        ui.track_slider(ID_WHEEL_V, &mut v, &RANGE_WV, Some(&tk));
        self.whsv[2] = v as u32;
        ui.box_end();

        ui.text(TS_BODY_STRONG, "Farbwähler");
        ui.color_picker_ex(ID_PICKER, &mut self.pick, CP_ALPHA | CP_FIELDS);
    }

    fn tracks(&mut self, ui: &mut Ui) {
        const RANGES: [&Range; 7] = [&RANGE_HUE, &RANGE_PCT, &RANGE_PCT, &RANGE_BYTE, &RANGE_BYTE, &RANGE_BYTE, &RANGE_BYTE];
        self.build();
        ui.box_begin(0, BOX_H, 12);
        ui.next_size(SIZE_FILL, 0);
        ui.box_begin(0, BOX_V, 4);
        for i in 0..7 {
            let mut v = self.trk[i];
            let tk = self.tk[i].clone();
            ui.next_size(SIZE_FILL, 36);
            let ev = ui.track_slider(ID_H + i as u32, &mut v, RANGES[i], Some(&tk));
            self.trk[i] = v;
            if ev & EV_ACT != 0 {
                if i < 3 {
                    self.hsv_changed();
                } else if i < 6 {
                    self.rgb_changed();
                }
            }
        }
        ui.box_end();
        ui.box_begin(0, BOX_V | BOX_XCENTER, 8);
        ui.next_size(40, 180);
        let tk = self.tk[2].clone();
        if ui.track_slider(ID_VT, &mut self.trk[2], &RANGE_PCT_V, Some(&tk)) & EV_ACT != 0 {
            self.hsv_changed();
        }
        ui.next_size(48, 48);
        if ui.swatch(ID_SW, self.argb, false) & EV_ACT != 0 {
            ui.menu_popup(&M_SWATCH);
        }
        ui.context_menu(&M_SWATCH);
        ui.box_end();
        ui.box_end();
        let kept = if self.kept == 0 { alloc::string::String::from("–") } else { alloc::format!("#{:08X}", self.kept) };
        ui.text(TS_BODY | TX_SECONDARY, &alloc::format!("Gemerkt: {kept}"));
    }

    fn hsv_changed(&mut self) {
        let t = &mut self.trk;
        let c = hsv_to_rgb((t[0] * 10) as u32, (t[1] * 10) as u32, (t[2] * 10) as u32);
        t[3] = ((c >> 16) & 255) as i32;
        t[4] = ((c >> 8) & 255) as i32;
        t[5] = (c & 255) as i32;
        self.build();
    }

    fn rgb_changed(&mut self) {
        let t = &mut self.trk;
        let rgb = (t[3] as u32) << 16 | (t[4] as u32) << 8 | t[5] as u32;
        let (h, sa, v) = rgb_to_hsv(rgb);
        if sa != 0 {
            let hh = (h + 5) / 10;
            t[0] = if hh >= 360 { 0 } else { hh as i32 };
        }
        t[1] = ((sa + 5) / 10) as i32;
        t[2] = ((v + 5) / 10) as i32;
        self.build();
    }

    // the colour and the stops of the S, V, R, G, B, A tracks
    fn build(&mut self) {
        let t = self.trk;
        let rgb = (t[3] as u32) << 16 | (t[4] as u32) << 8 | t[5] as u32;
        self.argb = (t[6] as u32) << 24 | rgb;
        let (h, sa, v) = ((t[0] * 10) as u32, (t[1] * 10) as u32, (t[2] * 10) as u32);
        self.tk[1].stops[0] = hsv_to_rgb(h, 0, v);
        self.tk[1].stops[1] = hsv_to_rgb(h, 1000, v);
        self.tk[2].stops[0] = 0xFF00_0000;
        self.tk[2].stops[1] = hsv_to_rgb(h, sa, 1000);
        let c = (rgb & 0x00FFFF) | 0xFF00_0000;
        self.tk[3].stops[0] = c;
        self.tk[3].stops[1] = c | 0x00FF_0000;
        let c = (rgb & 0xFF00FF) | 0xFF00_0000;
        self.tk[4].stops[0] = c;
        self.tk[4].stops[1] = c | 0x0000_FF00;
        let c = (rgb & 0xFFFF00) | 0xFF00_0000;
        self.tk[5].stops[0] = c;
        self.tk[5].stops[1] = c | 0x0000_00FF;
        self.tk[6].stops[0] = rgb;
        self.tk[6].stops[1] = rgb | 0xFF00_0000;
    }

    pub fn command(&mut self, id: u32) -> bool {
        if id == ID_COPY {
            self.kept = self.argb;
            return true;
        }
        false
    }
}
