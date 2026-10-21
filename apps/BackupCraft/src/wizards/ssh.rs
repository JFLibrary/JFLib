// ssh -- install an agent over SSH: address, the host key's fingerprint (checked by the user
// before anything is sent), credentials (stored or entered once), shared folders, then the
// server uploads the agent, installs the service and pairs it

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{self, Method};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use ui::*;

use crate::app::Console;
use crate::net::After;
use crate::tr::{t, tf};
use crate::wizards::{page_begin, page_end, ID_WSSH};
use crate::widgets;

pub struct SshWiz {
    step: usize,
    address: String,
    port: i32,
    name: String,
    fp: Option<api::SshFingerprint>,
    checked: i32,
    cred: i32,
    user: String,
    password: String,
    roots: String,
    pub busy: bool,
    pub error: String,
}

impl Default for SshWiz {
    fn default() -> SshWiz {
        SshWiz {
            step: 0,
            address: String::new(),
            port: 22,
            name: String::new(),
            fp: None,
            checked: 0,
            cred: 0,
            user: String::from("root"),
            password: String::new(),
            roots: String::from("/home\n/etc"),
            busy: false,
            error: String::new(),
        }
    }
}

const STEPS: usize = 5;

impl SshWiz {
    pub fn done(&mut self, label: &str, body: &Value) -> bool {
        self.busy = false;
        match label {
            "ssh.fingerprint" => {
                self.fp = api::SshFingerprint::from_json(body);
                self.step = 1;
                false
            }
            _ => true,
        }
    }

    fn check(&self, step: usize) -> &'static str {
        match step {
            0 if self.address.trim().is_empty() => t("Bitte die Adresse des Rechners angeben."),
            1 if self.fp.is_none() => t("Der Host-Schlüssel ist noch nicht abgefragt."),
            1 if self.checked == 0 => t("Bitte bestätigen, dass der Fingerabdruck stimmt."),
            2 if self.cred == 0 && (self.user.trim().is_empty() || self.password.is_empty()) => t("Bitte Benutzer und Passwort angeben oder gespeicherte Zugangsdaten wählen."),
            3 if self.roots.trim().is_empty() => t("Bitte mindestens einen Ordner freigeben."),
            _ => "",
        }
    }

    pub fn frame(&mut self, app: &mut Console, ui: &mut Ui) -> bool {
        let id = ID_WSSH;
        let steps = [t("Rechner"), t("Host-Schlüssel"), t("Anmeldung"), t("Freigaben"), t("Zusammenfassung")];
        let states: Vec<i32> = (0..STEPS).map(|i| if i < self.step { WS_DONE } else { WS_TODO }).collect();
        let (_, close) = page_begin(ui, id, t("Agent per SSH installieren"), t("Für Linux-Rechner mit OpenSSH (Ed25519-Host-Schlüssel)."), &steps, self.step, &states);
        let now = ui.now();
        match self.step {
            0 => {
                ui.text(TS_SUBTITLE, t("Rechner"));
                widgets::label(ui, t("Adresse"));
                ui.next_size(420, 0);
                ui.textbox(id + 0x200, Some("server.example.local"), &mut self.address, 255);
                widgets::label(ui, t("SSH-Port"));
                ui.next_size(120, 0);
                ui.number_box(id + 0x201, &mut self.port, &Range::new(1, 65535, 1, 0));
                widgets::label(ui, t("Anzeigename (optional)"));
                ui.next_size(420, 0);
                ui.textbox(id + 0x202, None, &mut self.name, 120);
            }
            1 => {
                ui.text(TS_SUBTITLE, t("Host-Schlüssel prüfen"));
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_WRAP, t("Vergleichen Sie den Fingerabdruck mit dem des Rechners (z. B. „ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub“). Erst nach Ihrer Bestätigung sendet der Server Zugangsdaten."));
                if let Some(fp) = &self.fp {
                    ui.box_begin(0, BOX_V | BOX_LAYER | pad(16), 6);
                    ui.text(TS_CAPTION | TX_SECONDARY, &format!("{}:{} · {}", fp.address, fp.port, fp.key_type));
                    ui.text(TS_SUBTITLE, &fp.fingerprint);
                    ui.box_end();
                    ui.checkbox(id + 0x300, t("Der Fingerabdruck stimmt mit dem des Rechners überein."), &mut self.checked);
                }
            }
            2 => {
                ui.text(TS_SUBTITLE, t("Anmeldung"));
                app.net.want("/api/credentials", 30_000, now);
                let creds: Vec<Credential> = app.net.data.creds.clone();
                let mut names: Vec<String> = alloc::vec![String::from(t("Benutzer und Passwort eingeben"))];
                names.extend(creds.iter().map(|c| format!("{} ({})", c.label, c.username)));
                let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
                ui.next_size(420, 0);
                ui.combobox(id + 0x400, &mut self.cred, &refs);
                if self.cred == 0 {
                    widgets::label(ui, t("Benutzer"));
                    ui.next_size(420, 0);
                    ui.textbox(id + 0x401, None, &mut self.user, 64);
                    widgets::label(ui, t("Passwort"));
                    ui.next_size(420, 0);
                    ui.textbox(id + 0x402, None, &mut self.password, 256 | TB_PASSWORD);
                    widgets::hint(ui, t("Das Passwort wird nur für diese Installation verwendet und nicht gespeichert."));
                }
            }
            3 => {
                ui.text(TS_SUBTITLE, t("Freigegebene Ordner"));
                widgets::hint(ui, t("Nur aus diesen Ordnern darf der Server später lesen (einer pro Zeile)."));
                ui.next_size(420, 120);
                ui.textbox(id + 0x500, None, &mut self.roots, 4096 | TB_MULTI);
            }
            _ => {
                ui.text(TS_SUBTITLE, t("Zusammenfassung"));
                let fp = self.fp.as_ref().map_or(String::new(), |f| f.fingerprint.clone());
                let roots = self.roots.lines().filter(|l| !l.trim().is_empty()).count();
                widgets::props(ui, &[
                    (t("Rechner"), &format!("{}:{}", self.address, self.port)),
                    (t("Host-Schlüssel"), &fp),
                    (t("Freigaben"), &tf("{} Ordner", &[&format!("{}", roots)])),
                ]);
                ui.infobar(id + 0x600, IB_INFO, t("Ablauf"), t("Upload des Agenten, Einrichtung als Dienst (systemd), Kopplung mit eigenem Zertifikat."));
            }
        }
        let err = self.check(self.step);
        let r = page_end(ui, id + 0x100, self.step == 0, self.step + 1 == STEPS, err.is_empty() || (self.step == 0 && !self.address.trim().is_empty()),
                         self.busy, &self.error.clone(), t("Installieren"));
        match r {
            1 => self.step = self.step.saturating_sub(1),
            2 if self.step == 0 => {
                self.busy = true;
                self.error.clear();
                let body = j::obj([("address", Value::string(self.address.trim())), ("port", Value::u64(self.port as u64))]);
                app.net.action(Method::Post, "/api/hosts/ssh-fingerprint", Some(body), "ssh.fingerprint", &[], After::SshFingerprint);
            }
            2 if self.step + 1 < STEPS => self.step += 1,
            2 => {
                self.busy = true;
                let creds = app.net.data.creds.clone();
                let cred_id = if self.cred > 0 { creds.get(self.cred as usize - 1).map_or(String::new(), |c| c.id.clone()) } else { String::new() };
                let body = api::SshPushRequest {
                    address: String::from(self.address.trim()),
                    port: self.port as u32,
                    name: String::from(self.name.trim()),
                    host_fingerprint: self.fp.as_ref().map_or(String::new(), |f| f.fingerprint.clone()),
                    credential_id: cred_id,
                    username: String::from(self.user.trim()),
                    password: core::mem::take(&mut self.password),
                    roots: self.roots.lines().map(|l| String::from(l.trim())).filter(|l| !l.is_empty()).collect(),
                }
                .to_json();
                app.net.action(Method::Post, "/api/hosts/ssh-push", Some(body), "ssh.push", &["/api/hosts"], After::CloseWizard);
            }
            3 => return false,
            _ => {}
        }
        !close
    }
}
