// staged -- the pages of features that are prepared but not available yet (replication,
// tape, instant start / boot checks): what they will do, the server's capability state,
// the commands (disabled) and the empty views they will fill

use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api;
use ui::*;

use crate::app::{Console, ID_STAGED};
use crate::icons::*;
use crate::tr::t;
use crate::{views, widgets};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Replication,
    Tape,
    Recovery,
}

struct Info {
    title: &'static str,
    sub: &'static str,
    icon: u32,
    caps: &'static [&'static str],
    points: &'static [&'static str],
    cmds: &'static [(u32, &'static str)],
    cols: &'static [&'static str],
}

fn info(k: Kind) -> Info {
    match k {
        Kind::Replication => Info {
            title: t("Replikation"),
            sub: t("Replikate von Systemen auf einem Zielhost, Failover-Pläne und Failback."),
            icon: BC_REPLICATE,
            caps: &[api::CAP_REPLICATION, api::CAP_CDP],
            points: &[
                "Replikat vorbereiten: ein startbereiter Stand auf einem Zielhost, aus jedem neuen Restore-Point nachgeführt.",
                "Failover-Plan: Reihenfolge, Wartezeiten und Prüfungen für den Ernstfall.",
                "Failback: die Änderungen während des Failovers zurück auf das Original übertragen.",
                "Datei-CDP: Änderungen fortlaufend aufzeichnen, Rückkehr zu jedem Zeitpunkt.",
            ],
            cmds: &[(IC_ADD, "Replikat anlegen …"), (BC_REPLICATE, "Failover-Plan …"), (IC_PLAY, "Failover"), (IC_BACK, "Failback")],
            cols: &["Replikat", "Quelle", "Zielhost", "Stand", "Zustand"],
        },
        Kind::Tape => Info {
            title: t("Tape"),
            sub: t("Medienpools, Bänder und Auslagerung – auch als virtuelle Bandbibliothek auf Platte."),
            icon: BC_TAPE,
            caps: &[api::CAP_TAPE],
            points: &[
                "Medienpools mit eigener Aufbewahrung und Schreibschutz.",
                "Restore-Points vollständig auf Band schreiben – lesbar auch ohne Repository.",
                "Katalog, Prüfung und Wiederherstellung direkt vom Band.",
            ],
            cmds: &[(IC_ADD, "Medienpool anlegen …"), (BC_TAPE, "Band hinzufügen …"), (IC_UPLOAD, "Auf Band schreiben"), (BC_SHIELD_CHECK, "Band prüfen")],
            cols: &["Band", "Pool", "Belegt", "Schutz bis", "Zustand"],
        },
        Kind::Recovery => Info {
            title: t("Sofortstart und Startprüfung"),
            sub: t("Systeme direkt aus einem Restore-Point starten und Sicherungen automatisch auf Startfähigkeit prüfen."),
            icon: BC_VM,
            caps: &[api::CAP_INSTANT, api::CAP_BOOTCHECK, api::CAP_HYPERVISOR],
            points: &[
                "Sofortstart: eine VM aus dem Restore-Point starten, ohne vorher alles zurückzukopieren (QEMU/KVM, Hyper-V).",
                "Zusammenführen oder verwerfen: Änderungen während des Betriebs übernehmen oder rückstandslos entfernen.",
                "Startprüfung: Start in einer isolierten Umgebung, Gastprüfungen, Bericht.",
            ],
            cmds: &[(IC_PLAY, "VM starten …"), (IC_STOP, "Stoppen"), (IC_ACCEPT, "Zusammenführen"), (BC_SHIELD_CHECK, "Startprüfung …")],
            cols: &["Sitzung", "Restore-Point", "Hypervisor", "Gestartet", "Zustand"],
        },
    }
}

pub fn frame(app: &mut Console, ui: &mut Ui, kind: Kind) {
    let now = ui.now();
    app.net.want("/api/capabilities", 60_000, now);
    let inf = info(kind);
    views::scroll_body_begin(ui, ID_STAGED + 1);
    widgets::page_head(ui, inf.title, inf.sub);
    widgets::page_head_end(ui);
    let available = inf.caps.iter().any(|c| app.net.data.capable(c));
    if !available {
        ui.infobar(ID_STAGED + 2, IB_INFO, t("In Vorbereitung"),
                   t("Datenmodell, Schnittstelle und Oberfläche stehen bereit; die Ausführung folgt in einer der nächsten Versionen."));
    }
    let cmds: Vec<CmdItem> = inf.cmds.iter().enumerate().map(|(i, (icon, label))| {
        CmdItem::new(ID_STAGED + 10 + i as u32, *icon, t(label)).enabled(available).flags(if i == 0 { ST_ACCENT } else { 0 })
    }).collect();
    ui.command_bar(ID_STAGED + 30, &cmds);
    ui.box_begin(0, BOX_H, 16);
    ui.next_size(-3, 0);
    widgets::card_begin(ui, t("Was hier entsteht"), "");
    for p in inf.points {
        ui.box_begin(0, BOX_H, 10);
        ui.icon(IC_CHECK_MARK, 16);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, t(p));
        ui.box_end();
    }
    widgets::card_end(ui);
    ui.next_size(-2, 0);
    widgets::card_begin(ui, t("Status auf diesem Server"), "");
    for c in inf.caps {
        let cap = app.net.data.caps.iter().find(|x| x.name == *c).cloned();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
        let ok = cap.as_ref().is_some_and(|x| x.available);
        ui.status_dot(if ok { BADGE_OK } else { BADGE_NEUTRAL });
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 0);
        ui.text(TS_BODY, cap_name(c));
        let reason = cap.map_or(String::from(t("Unbekannt")), |x| if x.available { String::from(t("Verfügbar")) } else { x.reason });
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &reason);
        ui.box_end();
        ui.box_end();
    }
    widgets::card_end(ui);
    ui.box_end();
    ui.next_size(-1, 280);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols: Vec<GridCol> = inf.cols.iter().enumerate().map(|(i, c)| {
        let g = GridCol::new(t(c), 160);
        if i == 0 { g.flags(GC_FILL) } else { g }
    }).collect();
    let mut st = GridState::default();
    ui.next_size(-1, 52);
    let (_a, _b) = ui.grid_begin(ID_STAGED + 0x8_0000, &mut st, &cols, &[]);
    ui.grid_end(&mut st);
    ui.next_size(-1, -1);
    ui.empty_state(inf.icon, t("Noch keine Einträge"), t("Sobald die Funktion verfügbar ist, erscheinen die Einträge hier."));
    ui.box_end();
    views::scroll_body_end(ui);
}

fn cap_name(c: &str) -> &'static str {
    match c {
        api::CAP_REPLICATION => t("Replikation und Failover"),
        api::CAP_CDP => t("Datei-CDP"),
        api::CAP_TAPE => t("Bandbibliothek"),
        api::CAP_INSTANT => t("Sofortstart"),
        api::CAP_BOOTCHECK => t("Startprüfung"),
        api::CAP_HYPERVISOR => t("Hypervisor-Anbindung"),
        _ => t("Funktion"),
    }
}
