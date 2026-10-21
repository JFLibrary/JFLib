// creds -- stored credentials (SSH installation of agents, network shares): add and delete;
// the secrets stay on the server, sealed with its master key

use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::ids;
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_CREDS};
use crate::icons::*;
use crate::tr::{t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets};

#[derive(Default)]
pub struct State {
    pub grid: GridState,
}

const ID_CB: u32 = ID_CREDS + 10;
const C_NEW: u32 = ID_CB;
const C_DELETE: u32 = ID_CB + 1;
const ID_GRID: u32 = ID_CREDS + 0x8_0000;

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    let admin = app.may(Role::Admin);
    if admin {
        app.net.want("/api/credentials", 15_000, now);
    }
    let creds: Vec<Credential> = app.net.data.creds.clone();
    let keys: Vec<u64> = creds.iter().map(|c| ids::key(&c.id)).collect();
    app.creds.grid.retain(&keys);
    let sel: Vec<Credential> = creds.iter().filter(|c| app.creds.grid.sel.contains(&ids::key(&c.id))).cloned().collect();
    views::body_begin(ui);
    widgets::page_head(ui, t("Zugangsdaten"), t("Für die Installation von Agenten per SSH und für Netzwerkfreigaben. Die Geheimnisse verlassen den Server nie."));
    widgets::page_head_end(ui);
    if !admin {
        ui.infobar(ID_CREDS + 1, IB_INFO, t("Nur für Administratoren"), t("Zugangsdaten sehen und verwalten nur Administratoren."));
        views::body_end(ui);
        return;
    }
    let clicked = ui.command_bar(ID_CB + 20, &[
        CmdItem::new(C_NEW, IC_ADD, t("Zugangsdaten hinzufügen …")).flags(ST_ACCENT),
        CmdItem::new(C_DELETE, IC_DELETE, t("Löschen")).enabled(!sel.is_empty()).flags(CBI_SEP),
    ]);
    match clicked {
        Some(C_NEW) => app.dlg.open_credential(),
        Some(C_DELETE) => {
            let names: Vec<String> = sel.iter().map(|c| c.label.clone()).collect();
            app.dlg.confirm(Confirm::DeleteCreds(sel.iter().map(|c| c.id.clone()).collect()), t("Zugangsdaten löschen?"),
                            &tf("{} wird gelöscht. Installationen, die sie verwenden, müssen neue Zugangsdaten bekommen.", &[&names.join(", ")]), t("Löschen"));
        }
        _ => {}
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if creds.is_empty() && app.net.loaded("/api/credentials") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_KEY, t("Keine Zugangsdaten gespeichert"), t("Agenten lassen sich auch ganz ohne gespeicherte Zugangsdaten koppeln (Pairing-Code)."));
    } else {
        let cols = [GridCol::new(t("Bezeichnung"), 240).flags(GC_FILL), GridCol::new(t("Benutzer"), 200), GridCol::new(t("Art"), 160), GridCol::new(t("Angelegt"), 150)];
        let (a, b) = ui.grid_begin(ID_GRID, &mut app.creds.grid, &cols, &keys);
        for i in a..b {
            let c = &creds[i as usize];
            let st = &mut app.creds.grid;
            ui.grid_row(st, i);
            ui.grid_cell(st, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(if c.kind == CredKind::SshKey { BC_KEY } else { IC_LOCK }, 16);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &c.label);
            ui.box_end();
            ui.grid_text_ts(st, 1, TS_BODY | TX_SECONDARY, &c.username);
            ui.grid_text_ts(st, 2, TS_BODY | TX_SECONDARY, if c.kind == CredKind::SshKey { t("SSH-Schlüssel") } else { t("Passwort") });
            ui.grid_text_ts(st, 3, TS_BODY | TX_SECONDARY, &fmt::when(c.created, &app.zone));
        }
        ui.grid_end(&mut app.creds.grid);
    }
    ui.box_end();
    views::body_end(ui);
}
