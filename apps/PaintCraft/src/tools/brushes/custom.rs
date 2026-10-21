//! Native bitmap tips and per-dab dynamics, with checked external preset import.
use alloc::{string::String, sync::Arc, vec::Vec};
use ui::*;
use crate::app::Pn;

pub(crate) struct Tip { pub w: usize, pub h: usize, pub alpha: Vec<u8> }
#[derive(Clone)]
pub(crate) struct Dynamics {
    pub tip: Option<Arc<Tip>>, pub angle: i32, pub scatter: i32,
    pub size_jitter: i32, pub angle_jitter: i32, pub flow: i32, pub follow: i32,
    pub accumulate: bool, pub flow_byte: Option<u8>,
    pub opacity_cap: u8, pub density: u32,
}
impl Default for Dynamics {
    fn default() -> Self { Self { tip: None, angle: 0, scatter: 0, size_jitter: 0, angle_jitter: 0, flow: 100, follow: 0, accumulate: false, flow_byte: None, opacity_cap: 255, density: 0 } }
}
pub(crate) struct Custom { pub dynamics: Dynamics, pub path: String, pub presets: super::preset_ui::Library, candidate: String, open: Shared<i32>, pick: Shared<i32>, initialized: bool, rect:Shared<WinRect> }
impl Default for Custom {
    fn default()->Self { Self { dynamics:Default::default(),path:String::new(),presets:Default::default(),candidate:String::new(),open:Shared::new(0),pick:Shared::new(0),initialized:false,rect:Shared::new(WinRect::new(0,0,540,0,0)) } }
}
pub(crate) fn button(p: &mut Pn, ui: &mut Ui) {
    ui.next_size(28,0);
    if ui.icon_button(0x90200, IC_BRUSH, crate::preferences::localization::tr("Pinselspitze…")) & EV_ACT != 0 { p.custom_brush.open.set(1); ui.repass(); }
}
pub(crate) fn load(custom: &mut Custom) -> bool {
    if custom.path.is_empty() { custom.dynamics.tip = None; return true; }
    let data = if let Some(data) = super::preset_ui::builtin(&custom.path) { Vec::from(data) } else {
        let Some(data) = plat::read_file(&custom.path) else { return false; }; data
    };
    let Ok(sf) = img_core::io::decode_ex(&data, Some(&img_core::ImgDec { flags: img_core::IMGD_STRAIGHT, maxpx: 4_194_304, maxmem: 64_000_000, ..Default::default() }), None) else { return false; };
    let (w, h) = (sf.w as usize, sf.h as usize);
    let has_alpha = (0..h).any(|y| (0..w).any(|x| sf.px[y * sf.stride as usize + x * 4 + 3] != 255));
    let mut alpha = Vec::with_capacity(w*h);
    for y in 0..h { for x in 0..w {
        let p = &sf.px[y * sf.stride as usize+x*4..];
        // Transparent image: use alpha; opaque image: black ink on white paper.
        alpha.push(if has_alpha { p[3] } else { 255-((p[2] as u32*54+p[1] as u32*183+p[0] as u32*19+128)>>8) as u8 });
    } }
    custom.dynamics.tip = Some(Arc::new(Tip { w, h, alpha })); true
}
fn set_path(custom: &mut Custom, path: String) -> bool {
    let old=core::mem::replace(&mut custom.path,path);
    if load(custom) { true } else { custom.path=old; false }
}
pub(crate) fn frame(p: &mut Pn, ui: &mut Ui) {
    if !p.custom_brush.initialized {
        p.custom_brush.initialized = true;
        if let Some(path) = plat::arg_value("--brush-tip") {
            if !set_path(&mut p.custom_brush,path) { crate::integrations::file_io::fio_error(p, Some("Pinselspitze"), "Die Pinselspitze ließ sich nicht laden (höchstens 4.194.304 Pixel)."); }
        }
        super::preset_ui::initialize(p, ui);
    }
    super::preset_ui::file_dialog(p, ui);
    let pick = p.custom_brush.pick.clone();
    if ui.filedlg(0x92000, FD_OPEN, &pick, &mut p.custom_brush.candidate, 4096, &["png,bmp,tif,tiff=Bild-Pinselspitzen"]) & EV_ACT != 0 {
        let candidate=p.custom_brush.candidate.clone();
        if !set_path(&mut p.custom_brush,candidate) { crate::integrations::file_io::fio_error(p, Some("Pinselspitze"), "Die Pinselspitze ließ sich nicht laden."); }
    }
    let open = p.custom_brush.open.clone();
    let rect=p.custom_brush.rect.clone();
    if ui.window_begin(0x60E2, crate::preferences::localization::tr("Pinselspitze und Dynamik"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        ui.text(TS_BODY | TX_WRAP, crate::preferences::localization::tr("Bildspitzen verwenden den Alphakanal oder bei deckenden Bildern die umgekehrte Helligkeit. Breite, Abstand, Farbe, Auswahl und Stiftdruck gelten wie beim runden Pinsel."));
        ui.text(TS_BODY|TX_WRAP, if p.custom_brush.path.is_empty() { crate::preferences::localization::tr("Runde Standardspitze") } else { &p.custom_brush.path });
        if ui.button(0x90201, crate::preferences::localization::tr("Bildspitze laden…")) & EV_ACT != 0 { p.custom_brush.candidate=p.custom_brush.path.clone(); p.custom_brush.pick.set(1); ui.repass(); }
        if ui.button(0x90202, crate::preferences::localization::tr("Runde Spitze / Zurücksetzen")) & EV_ACT != 0 { p.custom_brush.dynamics = Default::default(); p.custom_brush.path.clear(); }
        super::preset_ui::controls(p, ui);
        let d = &mut p.custom_brush.dynamics;
        for (id, label, v, max) in [(0x90203, "Drehwinkel (°)", &mut d.angle, 359), (0x90204, "Streuung (%)", &mut d.scatter, 200),
            (0x90205, "Größenvariation (%)", &mut d.size_jitter, 95), (0x90206, "Winkelvariation (°)", &mut d.angle_jitter, 180), (0x90207, "Deckkraft pro Abdruck (%)", &mut d.flow, 100)] {
            ui.box_begin(id+0x1000,BOX_H,8);
            ui.next_size(290,0); ui.text(TS_BODY, crate::preferences::localization::tr(label));
            ui.next_size(-1,0); if ui.number_box(id, v, &Range::new(0, max, 1, 0)) & EV_ACT != 0 && id == 0x90207 { d.flow_byte = None; }
            ui.box_end();
        }
        ui.checkbox(0x90208, crate::preferences::localization::tr("Spitze an Bewegungsrichtung ausrichten"), &mut d.follow);
        let mut accumulate = d.accumulate as i32;
        if ui.checkbox(0x90219, crate::preferences::localization::tr("Farbfluss bei überlappenden Abdrücken aufbauen"), &mut accumulate) & EV_ACT != 0 { d.accumulate = accumulate != 0; }
        let mut cap = d.opacity_cap as i32;
        ui.text(TS_BODY, crate::preferences::localization::tr("Maximale Strichdeckung (0–255)"));
        if ui.number_box(0x9021a, &mut cap, &Range::new(0,255,1,0)) & EV_ACT != 0 { d.opacity_cap = cap as u8; }
        if ui.button(0x90209, crate::preferences::localization::tr("Schließen")) & EV_ACT != 0 { p.custom_brush.open.set(0); crate::preferences::st_save(p, ui); ui.repass(); }
    }
    ui.window_end();
}

impl Tip {
    /// Bilinear coverage in normalized tip coordinates; zero outside the tip.
    pub fn sample(&self, u: f64, v: f64) -> u8 {
        let (x, y) = (u * self.w as f64 - 0.5, v * self.h as f64 - 0.5);
        let (ix, iy) = (floor(x), floor(y));
        let (fx, fy) = (x-ix as f64, y-iy as f64);
        let get = |x: i32,y: i32| if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h { self.alpha[y as usize*self.w+x as usize] as f64 } else { 0.0 };
        ((get(ix,iy)*(1.0-fx)+get(ix+1,iy)*fx)*(1.0-fy)+(get(ix,iy+1)*(1.0-fx)+get(ix+1,iy+1)*fx)*fy+0.5).clamp(0.0,255.0) as u8
    }
}

fn floor(v: f64) -> i32 { let i = v as i32; if i as f64 > v { i-1 } else { i } }
