// services -- the Dienste tab of a device window: the services the simulator runs on the device
// (DHCP, DNS, syslog, NTP, the TCP test services) with their state, status and numbers
//
//   tab(n, ui, k, id, dev)       a card per service; the project's services switch on and off
//                                and open their settings (svcedit.rs, in window k)
//
// A switch is one undo step that keeps the running device (Doc::apply_kept: the service
// starts or stops at once, and so it does again on undo and redo).  Services the device made
// for itself (from its configuration) show their state only.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::constants::ID_SVC_EDIT_BTN0;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;

/// the name of a kind of service (in the current language)
fn kind_name(kind: &str) -> &str {
    t(match kind {
        "dhcp-client" => "DHCP-Client",
        "dhcp-server" => "DHCP-Server",
        "dhcp-relay" => "DHCP-Relay",
        "dns-server" => "DNS-Server",
        "syslog-server" => "Syslog-Server",
        "ntp" => "Zeit (NTP)",
        "tcp-echo" => "TCP-Echo",
        "tcp-discard" => "TCP-Discard",
        "tcp-client" => "TCP-Testclient",
        "http-server" => "Webserver (HTTP)",
        "http-client" => "HTTP-Client",
        "http-load" => "Lastgenerator (HTTP)",
        "script" => "Skript",
        "lb" => "Loadbalancer",
        "kv" => "KV-Knoten",
        "cache" => "Cache",
        "mq" => "Message Queue",
        "worker" => "Worker",
        "rip" => "Routing (RIP)",
        "ospf" => "Routing (OSPF)",
        "tftp-server" => "TFTP-Server",
        "ftp-server" => "FTP-Server",
        "smtp-server" => "Mailserver (SMTP)",
        "pop3-server" => "Postfach (POP3)",
        _ => return kind,
    })
}

/// the Dienste tab of device window k (ids id .. id + 0x1F, the settings' buttons
/// ID_SVC_EDIT_BTN0 + 0x40 k + i)
pub(crate) fn tab(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let svcs = n.doc.sim.state().services(dev);
    let mut switch: Option<(usize, bool)> = None;
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 8);
    if svcs.is_empty() {
        ui.next_size(-1, 0);
        ui.text(
            TS_BODY | TX_SECONDARY | TX_WRAP,
            &tf("Auf {} läuft kein Dienst. Dienste (DHCP, DNS, Syslog, NTP) kommen aus der Projektdatei oder aus Einstellungen wie „Adresse per DHCP beziehen“.", &[&d.name]),
        );
    }
    for (i, s) in svcs.iter().enumerate().take(30) {
        let def = d.services.iter().position(|x| x.name.eq_ignore_ascii_case(&s.name));
        ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        let (badge, state) = match (s.enabled, s.running) {
            (_, true) => (BADGE_OK, t("läuft")),
            (true, false) => (BADGE_WARN, t("gestoppt")),
            (false, false) => (BADGE_NEUTRAL, t("aus")),
        };
        ui.status_dot(badge);
        ui.text(TS_BODY_STRONG, &format!("{} · {}", kind_name(s.kind), s.name));
        ui.text(TS_BODY | TX_SECONDARY, state);
        ui.spacer(-1);
        match def {
            Some(j) => {
                // its settings as JSON (svcedit.rs), then the switch
                super::svcedit::button(n, ui, ID_SVC_EDIT_BTN0 + 0x40 * k as u32 + i as u32, dev, j);
                let mut on = (!d.services[j].disabled) as i32;
                if ui.switch(id + 1 + i as u32, t("Eingeschaltet"), &mut on) & EV_ACT != 0 {
                    switch = Some((j, on != 0));
                }
            }
            None => ui.text(TS_CAPTION | TX_TERTIARY, t("aus der Konfiguration")),
        }
        ui.box_end();
        if !s.status_msg.is_empty() {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &crate::app::tr::msg(&s.status_msg));
        }
        let nums: Vec<String> = s.metrics.iter().map(|m| format!("{} {}", m.name, m.value)).collect();
        if !nums.is_empty() {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, &nums.join(" · "));
        }
        // the settings of a project's service while they are open (svcedit.rs)
        if let Some(j) = def {
            super::svcedit::editor(n, ui, dev, j);
        }
        ui.box_end();
    }
    ui.box_end();
    ui.scroll_end();
    // the switch, after the widgets: one undo step, the service starts or stops now
    if let Some((j, on)) = switch {
        let name = d.services[j].name.clone();
        let label = format!("Dienst {} auf {} {}", name, d.name, if on { "einschalten" } else { "ausschalten" });
        if let Some(e) = edits::device_edit(&n.doc.proj, dev, &label, |x| x.services[j].disabled = !on) {
            n.doc.apply_kept(e, dev);
        }
        ui.repass();
    }
}
