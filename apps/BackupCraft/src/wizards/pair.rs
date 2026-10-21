// pair -- pair an agent with a code: the server makes a short-lived code; on the machine the
// agent pairs with it (and checks the server's CA fingerprint); the new host appears here

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::j::{Json, Value};
use bc_proto::model::*;
use ui::*;

use crate::app::Console;
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::wizards::{page_begin, page_end, ID_WPAIR};
use crate::{fmt, widgets};

#[derive(Default)]
pub struct PairWiz {
    step: usize,
    code: Option<PairingCode>,
    os: i32,
    known: Vec<String>,
    pub busy: bool,
    pub error: String,
}

impl PairWiz {
    pub fn done(&mut self, body: &Value) -> bool {
        self.busy = false;
        if let Some(c) = PairingCode::from_json(body) {
            self.code = Some(c);
            self.step = 1;
        }
        false
    }

    pub fn frame(&mut self, app: &mut Console, ui: &mut Ui) -> bool {
        let id = ID_WPAIR;
        let now = ui.now();
        let epoch = app.epoch(ui);
        let steps = [t("Code erzeugen"), t("Auf dem Rechner koppeln")];
        let (_, close) = page_begin(ui, id, t("Agent koppeln"), t("Ohne gespeicherte Zugangsdaten: der Agent meldet sich mit einem Einmal-Code."), &steps, self.step, &[]);
        if self.known.is_empty() {
            self.known = app.net.data.hosts.iter().map(|h| h.id.clone()).collect();
            self.known.push(String::new());
        }
        match self.step {
            0 => {
                ui.text(TS_SUBTITLE, t("So funktioniert es"));
                for (n, line) in [
                    (1, t("BackupCraft-Agent auf dem Rechner installieren (Setup für Windows, .deb für Linux).")),
                    (2, t("Hier einen Pairing-Code erzeugen – gültig 15 Minuten, nur einmal verwendbar.")),
                    (3, t("Auf dem Rechner koppeln; der Agent zeigt den Fingerabdruck dieses Servers zum Vergleich.")),
                    (4, t("Der Server stellt dem Agenten ein eigenes Zertifikat aus; ab dann gilt gegenseitiges TLS.")),
                ] {
                    ui.box_begin(0, BOX_H, 8);
                    ui.next_size(18, 0);
                    ui.text(TS_BODY_STRONG | TX_ACCENT, &format!("{}.", n));
                    ui.next_size(-1, 0);
                    ui.text(TS_BODY | TX_WRAP, line);
                    ui.box_end();
                }
                widgets::label(ui, t("Betriebssystem des Rechners"));
                widgets::inline_begin(ui);
                ui.segmented(id + 0x200, &mut self.os, &["Linux", "Windows"]);
                widgets::inline_end(ui);
                widgets::inline_begin(ui);
                ui.next_icon(BC_PAIR);
                ui.next_flags(if self.busy { ST_DISABLED } else { 0 });
                if ui.accent_button(id + 0x208, t("Pairing-Code erzeugen")) & EV_ACT != 0 {
                    self.busy = true;
                    self.error.clear();
                    app.net.action(Method::Post, "/api/hosts/pairing", None, "pair.code", &[], After::Pairing);
                }
                widgets::inline_end(ui);
            }
            _ => {
                app.net.want("/api/hosts", 2000, now);
                let c = self.code.clone().unwrap_or_default();
                ui.text(TS_SUBTITLE, t("Pairing-Code"));
                ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(18), 16);
                ui.text(TS_TITLE_LARGE, &c.code);
                ui.box_begin(0, BOX_V, 2);
                ui.text(TS_BODY_STRONG, &tf("gültig bis {}", &[&datetime::format_time(datetime::Timestamp::from_secs(c.expires), &app.zone)]));
                let left = (c.expires - epoch).max(0);
                ui.text(TS_CAPTION | TX_SECONDARY, &tf("noch {}", &[&fmt::duration(left)]));
                ui.box_end();
                ui.spacer(-1);
                ui.next_icon(IC_COPY);
                if ui.button(id + 0x300, t("Kopieren")) & EV_ACT != 0 {
                    ui.clip_set(CLIP_TEXT, c.code.as_bytes());
                }
                ui.box_end();
                widgets::label(ui, t("Auf dem Rechner ausführen"));
                let cmd = if self.os == 1 {
                    format!("& \"C:\\Program Files\\BackupCraft\\bc-agent.exe\" pair --server {} --code {}", c.server_url, c.code)
                } else {
                    format!("sudo bc-agent pair --server {} --code {}", c.server_url, c.code)
                };
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.next_size(-1, 0);
                let mut cmd_box = cmd.clone();
                ui.textbox(id + 0x301, None, &mut cmd_box, 1024 | TB_READONLY);
                ui.next_icon(IC_COPY);
                ui.next_flags(ST_ICON_ONLY);
                if ui.button(id + 0x302, t("Befehl kopieren")) & EV_ACT != 0 {
                    ui.clip_set(CLIP_TEXT, cmd.as_bytes());
                    ui.toast(IB_SUCCESS, t("Kopiert"), t("Der Befehl liegt in der Zwischenablage."), 2000);
                }
                ui.box_end();
                widgets::label(ui, t("Fingerabdruck dieses Servers (zum Vergleich auf dem Rechner)"));
                ui.text(TS_BODY | TX_WRAP, &c.ca_fingerprint);
                // a host that appeared since the code was made
                let new: Vec<Host> = app.net.data.hosts.iter().filter(|h| !self.known.contains(&h.id)).cloned().collect();
                ui.box_begin(0, BOX_H | BOX_XCENTER | pad(6), 10);
                if let Some(h) = new.first() {
                    ui.icon(IC_CHECK_CIRCLE_FILL, 20);
                    ui.text(TS_BODY_STRONG, &tf("{} ist gekoppelt.", &[&h.name]));
                } else {
                    ui.spinner(id + 0x310, 18);
                    ui.text(TS_BODY | TX_SECONDARY, t("Warte auf den Agenten …"));
                }
                ui.box_end();
            }
        }
        let done = self.step == 1;
        let r = page_end(ui, id + 0x100, self.step == 0, done, done, self.busy, &self.error.clone(), t("Fertig"));
        match r {
            1 => self.step = 0,
            2 if done => return false,
            3 => return false,
            _ => {}
        }
        !close
    }
}
