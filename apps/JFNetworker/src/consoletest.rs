// consoletest -- checks of what the consoles build on (part of --selftest): the simulator's
// log lines as reports (IOS console messages with their severity, debugging, service logs), no
// renames from a booting configuration, a saved configuration as an undo step that keeps the
// running device (Doc::apply_kept), a switched service through undo and redo
//
// Devices of the small project get ids in order: PC1 = 1, R1 = 2, PC2 = 3; SRV1 = 4.

use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::{Action, Cmd, Engine, Report};

use crate::doc::{edits, Doc};
use crate::selftest::T;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

/// PC1 (10.0.1.10) - Gi0/0/0 R1 Gi0/0/1 - PC2 (10.0.2.10); SRV1 with a TCP echo service
fn project() -> Project {
    let mut p = Project::new("consoles", 5);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let srv = p.add_device("server-1u", 300, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(r1, "gi0/0/0"), None);
    let _ = p.connect(&pr(r1, "gi0/0/1"), &pr(pc2, "eth0"), None);
    let cfg = [
        (pc1, "hostname PC1\nip addr add 10.0.1.10/24 dev eth0\nip route add default via 10.0.1.1\n"),
        (r1, "hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.1.1 255.255.255.0\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ip address 10.0.2.1 255.255.255.0\n no shutdown\n!\nend\n"),
        (pc2, "hostname PC2\nip addr add 10.0.2.10/24 dev eth0\nip route add default via 10.0.2.1\n"),
        (srv, "hostname SRV1\nip addr add 10.0.2.20/24 dev eth0\n"),
    ];
    for (id, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.startup_config = String::from(c);
        }
    }
    if let Some(d) = p.device_mut(srv) {
        d.services.push(ServiceDef { kind: String::from("tcp-echo"), name: String::from("echo"), disabled: false, config: agent_core::jmap::empty_object() });
    }
    p
}

pub(crate) fn run(t: &mut T) {
    log_reports(t);
    boot_names(t);
    kept_save(t);
    kept_service(t);
}

/// the log lines of a device among reports: (severity, tag, text)
fn logs(rs: &[Report], dev: u32) -> Vec<(u8, String, String)> {
    rs.iter()
        .filter_map(|r| match r {
            Report::Log { dev: d, sev, tag, text, .. } if *d == dev => Some((*sev, tag.clone(), text.clone())),
            _ => None,
        })
        .collect()
}

fn log_reports(t: &mut T) {
    let p = project();
    let (pc1, r1) = (1, 2);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let _ = e.take_reports();
    for l in ["enable", "conf t", "interface gi0/0/1", "shutdown", "end"] {
        let _ = e.cli_line(r1, l);
    }
    let rs = e.take_reports();
    let l = logs(&rs, r1);
    t.check("config message", l.iter().any(|x| *x == (5, String::from("%SYS-5-CONFIG_I"), String::from("configuration changed at the console"))));
    t.check("shut down", l.iter().any(|x| x.0 == 5 && x.1 == "%LINK-5-CHANGED" && x.2 == "GigabitEthernet0/0/1 was shut down"));
    t.check("line protocol", l.iter().any(|x| x.0 == 5 && x.1 == "%LINEPROTO-5-UPDOWN"));
    t.check("console shows them", rs.iter().any(|r| matches!(r, Report::CliOut { dev, bytes, .. } if *dev == r1 && String::from_utf8_lossy(bytes).contains("%LINK-5-CHANGED: GigabitEthernet0/0/1 was shut down"))));
    // debugging: severity 7, its subsystem as the source; not in the device's own log
    let _ = e.cli_line(r1, "debug ip icmp");
    let _ = e.cli_line(pc1, "ping -c 1 10.0.1.1");
    let t1 = e.now() + 5 * jfn_sim::SEC;
    e.run_until(t1, 1_000_000);
    let l = logs(&e.take_reports(), r1);
    t.check("debug line", l.iter().any(|x| x.0 == 7 && x.1 == "ICMP" && x.2 == "sent echo reply 10.0.1.1 -> 10.0.1.10"));
    t.check("debug stays local", !e.log_lines(r1).iter().any(|x| x.tag == "ICMP"));
    // a service's line (logger) comes as a report as well
    let _ = e.apply(Action::Log { dev: pc1, sev: 6, tag: String::from("logger"), text: String::from("hello") });
    let l = logs(&e.take_reports(), pc1);
    t.eq("service log", l, alloc::vec![(6, String::from("logger"), String::from("hello"))]);
}

// a startup configuration that boots renames nothing (the project keeps its name, Sim::fix_name);
// typed at the console, "hostname" is a rename the project follows
fn boot_names(t: &mut T) {
    let mut p = project();
    if let Some(d) = p.device_mut(2) {
        d.name = String::from("Edge"); // its configuration still says "hostname R1"
    }
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    t.check("no rename at boot", !e.take_reports().iter().any(|r| matches!(r, Report::Hostname { .. })));
    let _ = e.apply(Action::RemoveDevice(2));
    let _ = e.apply(Action::AddDevice(p.devices[1].clone()));
    t.check("no rename at a restart", !e.take_reports().iter().any(|r| matches!(r, Report::Hostname { .. })));
    for l in ["enable", "conf t", "hostname Core"] {
        let _ = e.cli_line(2, l);
    }
    t.check("typed: a rename", e.take_reports().iter().any(|r| matches!(r, Report::Hostname { dev: 2, name } if name == "Core")));
}

/// R1's startup configuration as the engine has it
fn startup(doc: &Doc, dev: u32) -> String {
    doc.sim.engine().startup_config(dev)
}

fn kept_save(t: &mut T) {
    let r1 = 2;
    let mut doc = Doc::new();
    doc.replace(project(), None);
    doc.sim.start();
    let old = doc.proj.device(r1).map(|d| d.startup_config.clone()).unwrap_or_default();
    // a change typed at the console, then "copy running-config startup-config"
    for l in ["enable", "conf t", "interface gi0/0/0", "description uplink", "end", "copy running-config startup-config", ""] {
        let _ = doc.sim.send(Cmd::Cli { dev: r1, line: String::from(l) });
    }
    let saved = doc.sim.take_reports().into_iter().find_map(|r| match r {
        Report::Startup { dev, text } if dev == r1 => Some(text),
        _ => None,
    });
    t.check("saved", saved.as_ref().is_some_and(|s| s.contains(" description uplink")));
    let text = saved.unwrap_or_default();
    if let Some(e) = edits::device_edit(&doc.proj, r1, "Konfiguration von R1 sichern", |x| x.startup_config = text.clone()) {
        doc.apply_kept(e, r1);
    }
    t.check("dirty", doc.dirty());
    t.eq("project has it", doc.proj.device(r1).map(|d| d.startup_config.clone()), Some(text.clone()));
    t.eq("engine has it", startup(&doc, r1), text.clone());
    // undo: the old startup configuration, the running device as it is (no restart)
    t.check("undo", doc.undo());
    t.eq("project back", doc.proj.device(r1).map(|d| d.startup_config.clone()), Some(old.clone()));
    t.eq("engine back", startup(&doc, r1), old.clone());
    t.check("still running it", doc.sim.engine().running_config(r1).contains(" description uplink"));
    t.eq("console where it was", doc.sim.engine().cli_prompt(r1), String::from("R1#"));
    t.check("redo", doc.redo());
    t.eq("engine again", startup(&doc, r1), text);
    t.check("running still", doc.sim.engine().running_config(r1).contains(" description uplink"));
}

fn kept_service(t: &mut T) {
    let srv = 4;
    let mut doc = Doc::new();
    doc.replace(project(), None);
    doc.sim.start();
    let enabled = |doc: &Doc| doc.sim.engine().services(srv).iter().find(|s| s.name == "echo").map(|s| s.enabled);
    t.eq("echo on", enabled(&doc), Some(true));
    // something only the running device knows: a restart would lose it
    let _ = doc.sim.send(Cmd::Cli { dev: srv, line: String::from("ip addr add 10.0.2.21/24 dev eth0") });
    if let Some(e) = edits::device_edit(&doc.proj, srv, "Dienst echo ausschalten", |x| x.services[0].disabled = true) {
        doc.apply_kept(e, srv);
    }
    t.eq("echo off", enabled(&doc), Some(false));
    t.check("undo", doc.undo());
    t.eq("echo on again", enabled(&doc), Some(true));
    t.check("redo", doc.redo());
    t.eq("echo off again", enabled(&doc), Some(false));
    t.check("no restart", doc.sim.engine().running_config(srv).contains("ip addr add 10.0.2.21/24 dev eth0"));
    t.check("the project keeps the switch", doc.proj.device(srv).is_some_and(|d| d.services[0].disabled));
}
