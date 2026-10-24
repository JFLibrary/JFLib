//! Preset library UI and transactional installation into the native brush engine.
use super::{custom, presets};
use crate::app::Pn;
use alloc::{format, string::String, vec::Vec};
use ui::*;

pub(crate) struct Library {
    path: String,
    candidate: String,
    pick: Shared<i32>,
    items: Vec<presets::Preset>,
    selected: i32,
    license: Shared<i32>,
    license_rect: Shared<WinRect>,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            path: String::new(),
            candidate: String::new(),
            pick: Shared::new(0),
            items: Vec::new(),
            selected: 0,
            license: Shared::new(0),
            license_rect: Shared::new(WinRect::new(0, 0, 680, 0, 0)),
        }
    }
}
pub(super) fn builtin(path: &str) -> Option<&'static [u8]> {
    Some(match path {
        "brush-factory:Circle Light" => {
            include_bytes!("../../../resources/brush-factory/BrCircle.png")
        }
        "brush-factory:Circle Hard" => {
            include_bytes!("../../../resources/brush-factory/BrCircleHard.png")
        }
        "brush-factory:Circle Med" => {
            include_bytes!("../../../resources/brush-factory/BrCircleMedium.png")
        }
        "brush-factory:Rough" => {
            include_bytes!("../../../resources/brush-factory/BrCircleRough.png")
        }
        "brush-factory:Segmented" => {
            include_bytes!("../../../resources/brush-factory/BrCircleSegmented.png")
        }
        "brush-factory:Sketchy" => {
            include_bytes!("../../../resources/brush-factory/BrCircleSketchy.png")
        }
        _ => return None,
    })
}
fn import(p: &mut Pn, path: String) -> Result<(), String> {
    let data = plat::read_file(&path)
        .ok_or_else(|| String::from("Die Preset-Datei ließ sich nicht lesen."))?;
    let items = presets::parse(&data)?;
    let library = &mut p.custom_brush.presets;
    library.path = path;
    library.items = items;
    library.selected = 0;
    Ok(())
}
fn apply(p: &mut Pn, ui: &mut Ui) -> Result<(), String> {
    let library = &p.custom_brush.presets;
    let preset = library
        .items
        .get(library.selected as usize)
        .ok_or_else(|| String::from("Kein Preset ausgewählt."))?;
    if !preset.unsupported.is_empty() {
        return Err(format!(
            "{}: Diese aktiven Einstellungen werden noch nicht unterstützt: {}",
            preset.name,
            preset.unsupported.join(", ")
        ));
    }
    let builtin_path = format!("brush-factory:{}", preset.image);
    let path = if builtin(&builtin_path).is_some() {
        builtin_path
    } else if !presets::image_is_local(&preset.image) {
        return Err(format!(
            "{}: Das Pinselbild muss relativ zum Ordner der Preset-Datei liegen: {}",
            preset.name, preset.image
        ));
    } else {
        format!(
            "{}/{}",
            library.path.rsplit_once(['/', '\\']).map_or(".", |v| v.0),
            preset.image
        )
    };
    let mut candidate = custom::Custom::default();
    candidate.path = path;
    if !custom::load(&mut candidate) {
        return Err(format!(
            "Pinselbild fehlt oder ist ungültig: {}",
            preset.image
        ));
    }
    candidate.dynamics.angle = preset.rotation.rem_euclid(360);
    candidate.dynamics.follow = preset.follow as i32;
    candidate.dynamics.flow = (preset.flow as i32 * 100 + 127) / 255;
    candidate.dynamics.flow_byte = Some(preset.flow);
    candidate.dynamics.accumulate = true;
    candidate.dynamics.opacity_cap = preset.opacity;
    candidate.dynamics.density = preset.density as u32;
    let (size, density, color, blend) = (preset.size, preset.density, preset.color, preset.blend);
    // Finish the old stroke only after validating every field and decoding the tip.
    crate::tools::tool_commit_live(p, ui);
    p.custom_brush.dynamics = candidate.dynamics;
    p.custom_brush.path = candidate.path;
    let cfg = &mut p.tool.cfg;
    cfg.width = size as f64;
    cfg.spacing = ((100 + density / 2) / density).max(1) as u32;
    cfg.hard = 100;
    cfg.aa = 1;
    cfg.smooth = 0;
    cfg.pressure = 0;
    cfg.fill = crate::app::constants::FS_SOLID as u32;
    cfg.blend = blend;
    cfg.ver = cfg.ver.wrapping_add(1);
    crate::workspace::palette::pal_set_primary(&mut p.pal, color | 0xff00_0000);
    ui.repass();
    Ok(())
}
fn report(p: &mut Pn, result: Result<(), String>) {
    if let Err(e) = result {
        crate::integrations::file_io::fio_error(p, Some("Pinsel-Presets"), &e);
    }
}
pub(super) fn initialize(p: &mut Pn, ui: &mut Ui) {
    if let Some(path) = plat::arg_value("--brush-presets") {
        let result = import(p, path).and_then(|_| {
            if let Some(name) = plat::arg_value("--brush-preset") {
                let library = &mut p.custom_brush.presets;
                library.selected = library
                    .items
                    .iter()
                    .position(|v| v.name == name)
                    .ok_or_else(|| format!("Preset nicht gefunden: {name}"))?
                    as i32;
                apply(p, ui)?;
            }
            Ok(())
        });
        report(p, result);
    }
}
pub(super) fn file_dialog(p: &mut Pn, ui: &mut Ui) {
    let library = &mut p.custom_brush.presets;
    let pick = library.pick.clone();
    if ui.filedlg(
        0x92500,
        FD_OPEN,
        &pick,
        &mut library.candidate,
        4096,
        &["json,xml=Brush Factory / Dynamic Draw"],
    ) & EV_ACT
        != 0
    {
        let path = library.candidate.clone();
        let result = import(p, path);
        report(p, result);
    }
    let open = p.custom_brush.presets.license.clone();
    let rect = p.custom_brush.presets.license_rect.clone();
    if ui.window_begin(
        0x60e3,
        "Brush Factory / Dynamic Draw – MIT",
        Some(&open),
        WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT,
        Some(&rect),
    ) {
        ui.text(
            TS_BODY | TX_WRAP,
            include_str!("../../../resources/brush-factory/LICENSE.txt"),
        );
        if ui.button(0x9021b, crate::preferences::localization::tr("Schließen")) & EV_ACT != 0 {
            open.set(0);
            ui.repass();
        }
    }
    ui.window_end();
}
pub(super) fn controls(p: &mut Pn, ui: &mut Ui) {
    if ui.button(
        0x90210,
        crate::preferences::localization::tr("Brush-Factory-Presets importieren…"),
    ) & EV_ACT
        != 0
    {
        p.custom_brush.presets.pick.set(1);
        ui.repass();
    }
    let library = &mut p.custom_brush.presets;
    if !library.items.is_empty() {
        let names: Vec<_> = library.items.iter().map(|p| p.name.as_str()).collect();
        ui.combobox(0x90211, &mut library.selected, &names);
        if let Some(preset) = library.items.get(library.selected as usize) {
            let text = if preset.unsupported.is_empty() {
                format!("{} px · Bild: {}", preset.size, preset.image)
            } else {
                format!("Noch nicht anwendbar: {}", preset.unsupported.join(", "))
            };
            ui.text(TS_BODY | TX_WRAP, &text);
        }
        if ui.button(
            0x90212,
            crate::preferences::localization::tr("Preset anwenden"),
        ) & EV_ACT
            != 0
        {
            let result = apply(p, ui);
            report(p, result);
        }
    }
    if ui.button(
        0x90218,
        crate::preferences::localization::tr("Lizenz der Brush-Factory-Spitzen"),
    ) & EV_ACT
        != 0
    {
        p.custom_brush.presets.license.set(1);
        ui.repass();
    }
}
