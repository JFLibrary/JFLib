// hardware -- the hardware of a device in the inspector, "Hardware simulieren" of the project (SA1,
// jfn_sim's res.rs)
//
//   Bufs                 the fields of the card, the device and figures they started from, why
//                        the last change was refused
//   device(n, ui, id)    the card of a host: its figures now, a preset from the catalog, cores,
//                        speed, RAM, swap, disk; "Übernehmen" with or without a reboot
//   project(n, ui)       the switch of the project with what it changes
//
// Every change is one undo step of the project (Device::hw, Project::metered); the simulator follows
// (doc/sim.rs: Action::Resize with its downtime, Action::Metered).  The switch "Im Betrieb" says
// which downtime the next change takes; an undo reboots.  Figures of the catalog are assumptions,
// and the card says so: learners take numbers as truth.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{self, CliStyle, DiskClass, Hw, F_HOST};
use jfn_sim::res::{HOST, RESIZE_DOWN};
use ui::*;

use super::cable::parse_decimal;
use super::{field_begin, row, rows_begin};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;

/// the fields of the card
#[derive(Default)]
pub(crate) struct Bufs {
    /// the device and the figures the fields started from
    of: (u32, Option<Hw>),
    cores: String,
    speed: String,
    ram: String,
    swap: String,
    disk_gb: String,
    disk: i32,
    preset: i32,
    live: i32,
    err: String,
}

/// the kinds of disk in the order of their list
const DISKS: [DiskClass; 4] = [DiskClass::Hdd, DiskClass::Sata, DiskClass::Nvme, DiskClass::NvmeFast];

/// "1,5" of tenths and the like: a number as a field shows it
fn dec(v: u64, unit: u64) -> String {
    let (w, f) = (v / unit, v % unit);
    if f == 0 {
        return w.to_string();
    }
    let sep = if crate::app::tr::en() { '.' } else { ',' };
    format!("{}{}{}", w, sep, f * 100 / unit / if f * 100 / unit % 10 == 0 { 10 } else { 1 })
}

fn fill(b: &mut Bufs, hw: &Hw) {
    b.cores = hw.cores.to_string();
    b.speed = dec(hw.speed_pm as u64, 10);
    b.ram = dec(hw.ram_mb, 1024);
    b.swap = dec(hw.swap_mb, 1024);
    b.disk_gb = hw.disk_gb.to_string();
    b.disk = DISKS.iter().position(|d| *d == hw.disk).unwrap_or(0) as i32;
}

/// a number of GiB in a field ("0,5") -> MiB, to the thousandth of a GiB
pub(super) fn mib_of_gib(s: &str) -> Option<u64> {
    parse_decimal(s, 1000, 1 << 34).map(|v| v * 1024 / 1000)
}

/// the figures in the fields -> Hw, or what is wrong with them
fn read(b: &Bufs) -> Result<Hw, &'static str> {
    let cores = b.cores.trim().parse::<u32>().ok().filter(|c| (1..=1024).contains(c)).ok_or("Die Zahl der Kerne geht von 1 bis 1024.")?;
    let speed = parse_decimal(&b.speed, 10, 100_000).filter(|&v| v > 0).ok_or("Das Tempo ist eine Zahl in Prozent des Referenzkerns, z. B. 100.")?;
    let ram = mib_of_gib(&b.ram).filter(|&v| v > 0).ok_or("Der RAM ist eine Zahl von GiB größer als 0, z. B. 16 oder 0,5.")?;
    let swap = mib_of_gib(&b.swap).ok_or("Der Swap ist eine Zahl von GiB, 0 für keinen.")?;
    let disk_gb = b.disk_gb.trim().parse::<u64>().ok().filter(|&v| v > 0).ok_or("Die Disk ist eine Zahl von GiB größer als 0.")?;
    Ok(Hw { cores, speed_pm: speed as u32, ram_mb: ram, swap_mb: swap, disk: DISKS.get(b.disk as usize).copied().unwrap_or_default(), disk_gb })
}

// the figures of the device's hardware now: how busy, how full
fn state_rows(n: &Net, ui: &mut Ui, id: u32) {
    let e = n.doc.sim.state();
    let Some(v) = e.res_view(id) else { return };
    let sep = ui.decimal_sep();
    let cpu = e.series_one(id, HOST, "cpu_pm").and_then(|s| s.points.last().map(|p| p.1)).unwrap_or(0);
    let pct = LcUnit::Num("%").text(cpu as f64 / 10.0, sep);
    let bytes = jfn_sim::disk::size_text;
    rows_begin(ui);
    row(ui, t("CPU"), &tf("{}, {} wartend", &[&pct, &v.waiting.to_string()]));
    row(ui, t("Speicher"), &tf("{} von {}", &[&bytes(v.mem), &bytes(v.hw.ram_mb << 20)]));
    if v.swap_used > 0 {
        row(ui, t("Im Swap"), &bytes(v.swap_used));
    }
    if v.oom_kills > 0 {
        row(ui, t("Beendet"), &tf("{} Dienste (Speicher voll)", &[&v.oom_kills.to_string()]));
    }
    ui.box_end();
}

/// the card of a host (network devices have no such figures)
pub(crate) fn device(n: &mut Net, ui: &mut Ui, id: u32) {
    let Some(d) = n.doc.proj.device(id).cloned() else { return };
    let Some(m) = catalog::model(&d.model) else { return };
    if !m.has(F_HOST) || m.cli != CliStyle::Linux {
        return;
    }
    let of_model = catalog::hw_of(m.id);
    let now = d.hw.clone().or_else(|| if n.doc.proj.metered { of_model.clone() } else { None });
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 8);
    ui.text(TS_BODY_STRONG, t("Hardware"));
    let Some(now) = now else {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ohne simulierte Hardware kosten CPU und RAM hier nichts. Sie gilt für alle Geräte mit „Hardware simulieren“ im Projekt oder nur für dieses:"));
        ui.box_begin(0, BOX_H | BOX_END, 8);
        if ui.button(ID_HW_OWN, t("Für dieses Gerät angeben")) & EV_ACT != 0 {
            let hw = of_model.unwrap_or(Hw { cores: 4, speed_pm: 1000, ram_mb: 8 << 10, swap_mb: 1 << 10, disk: DiskClass::Sata, disk_gb: 240 });
            set_hw(n, id, Some(hw), RESIZE_DOWN);
            ui.repass();
        }
        ui.box_end();
        ui.box_end();
        return;
    };
    state_rows(n, ui, id);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t(if d.hw.is_some() { "Eigene Angaben." } else { "Annahmen für dieses Modell, keine Messwerte eines echten Geräts." }));
    let b = &mut n.inspector.hw;
    if b.of.0 != id || b.of.1.as_ref() != Some(&now) {
        b.of = (id, Some(now.clone()));
        fill(b, &now);
        b.preset = 0;
        b.err.clear();
    }
    // a preset fills the fields
    let presets: Vec<&'static catalog::Model> = catalog::hw_presets().collect();
    let names: Vec<&str> = core::iter::once(t("Eigene Werte")).chain(presets.iter().map(|p| t(p.name))).collect();
    field_begin(ui, t("Vorlage"));
    ui.next_size(-1, 0);
    let mut sel = b.preset;
    if ui.combobox(ID_HW_PRESET, &mut sel, &names) & EV_ACT != 0 {
        b.preset = sel;
        if let Some(hw) = sel.checked_sub(1).and_then(|k| presets.get(k as usize)).and_then(|p| catalog::hw_of(p.id)) {
            fill(b, &hw);
        }
        ui.repass();
    }
    ui.box_end();
    let fields = [(ID_HW_CORES, t("Kerne"), ""), (ID_HW_SPEED, t("Tempo"), "%"), (ID_HW_RAM, t("RAM"), "GiB"), (ID_HW_SWAP, t("Swap"), "GiB"), (ID_HW_DISK_GB, t("Disk"), "GiB")];
    for (fid, key, unit) in fields {
        let buf = match fid {
            ID_HW_CORES => &mut b.cores,
            ID_HW_SPEED => &mut b.speed,
            ID_HW_RAM => &mut b.ram,
            ID_HW_SWAP => &mut b.swap,
            _ => &mut b.disk_gb,
        };
        field_begin(ui, key);
        ui.next_size(-1, 0);
        if ui.textbox(fid, None, buf, 16) & EV_ACT != 0 {
            b.preset = 0;
        }
        ui.text(TS_BODY | TX_SECONDARY, unit);
        ui.box_end();
    }
    let kinds: Vec<&str> = DISKS.iter().map(|k| t(k.title())).collect();
    field_begin(ui, t("Disk-Art"));
    ui.next_size(-1, 0);
    let mut k = b.disk;
    if ui.combobox(ID_HW_DISK, &mut k, &kinds) & EV_ACT != 0 {
        b.disk = k;
        b.preset = 0;
        ui.repass();
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Tempo: 100 % ist der Referenzkern, mit dem die Kosten der Dienste angegeben sind (ein Skriptbefehl 5 ns)."));
    let mut live = b.live;
    if ui.switch(ID_HW_LIVE, t("Im Betrieb ändern"), &mut live) & EV_ACT != 0 {
        b.live = live;
        ui.repass();
    }
    ui.next_size(-1, 0);
    let what = if live != 0 { t("Die neuen Werte gelten sofort, ohne Neustart.") } else { t("Das Gerät startet dafür neu und fällt 30 s aus; was auf der Disk liegt, bleibt.") };
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, what);
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if d.hw.is_some() && n.doc.proj.metered {
        if ui.button(ID_HW_RESET, t("Modellwerte")) & EV_ACT != 0 {
            let down = if live != 0 { 0 } else { RESIZE_DOWN };
            set_hw(n, id, None, down);
            ui.repass();
        }
    }
    if ui.button(ID_HW_APPLY, t("Übernehmen")) & EV_ACT != 0 {
        let b = &mut n.inspector.hw;
        match read(b) {
            Ok(hw) if hw == now => b.err.clear(),
            Ok(hw) => {
                b.err.clear();
                set_hw(n, id, Some(hw), if live != 0 { 0 } else { RESIZE_DOWN });
            }
            Err(e) => b.err = String::from(t(e)),
        }
        ui.repass();
    }
    ui.box_end();
    let err = n.inspector.hw.err.clone();
    if !err.is_empty() {
        crate::workspace::error_text(ui, &err, 0);
    }
    ui.box_end();
}

// other figures for a device: one undo step, the simulator takes them with this downtime
fn set_hw(n: &mut Net, id: u32, hw: Option<Hw>, down: u64) {
    if let Some(e) = edits::device_hw(&n.doc.proj, id, hw) {
        n.doc.sim.hw_down = down;
        n.doc.apply(e);
    }
}

/// the project's page: "Hardware simulieren" and what it changes
pub(crate) fn project(n: &mut Net, ui: &mut Ui) {
    let mut on = n.doc.proj.metered as i32;
    if ui.switch(ID_PROJ_METERED, t("Hardware simulieren"), &mut on) & EV_ACT != 0 {
        if let Some(e) = edits::metered(&n.doc.proj, on != 0) {
            n.doc.apply(e);
        }
        ui.repass();
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Server und Hosts bekommen CPU und RAM ihres Modells: Dienste warten auf freie Kerne, zu wenig Speicher bremst sie. Aus: Rechenzeit kostet nichts, wie bisher."));
}
