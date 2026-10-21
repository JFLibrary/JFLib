// config -- settings that change a device's configuration (the inspector's quick fields): they
// apply at once to the running device and land in its startup configuration, one undo step
//
//   apply(doc, dev, label, acts, services) -> Result<(), String>
//        the Actions run in the simulator; the device's new startup configuration is what the
//        engine writes for a scratch copy of it (booted from the project's configuration, the
//        Actions applied); `services` replaces its services (a DHCP client comes or goes)
//   address(doc, dev, iface, text) / dhcp(doc, dev, iface, on) / iface_up(doc, dev, name, up) /
//   gateway(doc, dev, text)         the quick settings as Actions
//
// The configuration stays in one form: the engine renders it (IOS-like or as shell commands)
// and reads it back at the next start, exactly as a console's "copy running-config
// startup-config" would.  The simulator is told that it runs the new configuration already
// (Sim::adopt), so it does not restart the device; undo and redo restart it with the other
// configuration.  An address replaces the interface's addresses and ends its DHCP client;
// DHCP replaces the static addresses.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::{Device, ServiceDef};
use jfn_sim::{Action, Cmd, Engine};
use netpkt::{Ipv4, Prefix4};

use super::{edits, Doc};

/// the kind and name of a device's DHCP client on an interface (svcact.rs)
pub(crate) const DHCP_CLIENT: &str = "dhcp-client";

/// the project's DHCP client of an interface
pub(crate) fn dhcp_service(d: &Device, iface: &str) -> Option<usize> {
    d.services.iter().position(|s| s.kind == DHCP_CLIENT && s.config.get("iface").and_then(|v| v.as_str()).is_some_and(|i| jfn_model::ifname::same(i, iface)))
}

/// run Actions on device `dev` and keep its configuration (and services) as the new startup
/// configuration: one undo step
pub(crate) fn apply(doc: &mut Doc, dev: u32, label: &str, acts: &[Action], services: Option<Vec<ServiceDef>>) -> Result<(), String> {
    let d = doc.proj.device(dev).cloned().ok_or_else(|| format!("Das Gerät {} gibt es nicht.", dev))?;
    // the configuration with the change: a scratch copy boots from the project's one
    let mut e = Engine::new(doc.proj.seed);
    e.apply(Action::AddDevice(Device { services: Vec::new(), ..d.clone() })).map_err(|x| x.to_string())?;
    // the project's name, whatever the configuration says
    let _ = e.apply(Action::Hostname { dev, name: d.name.clone() });
    for a in acts {
        e.apply(a.clone()).map_err(|x| x.to_string())?;
    }
    let text = e.running_config(dev);
    // the running device
    for a in acts {
        doc.sim.send(Cmd::Apply(a.clone())).map_err(|x| x.to_string())?;
    }
    let services = services.unwrap_or_else(|| d.services.clone());
    if let Some(e) = edits::device_edit(&doc.proj, dev, label, |x| {
        x.startup_config = text;
        x.services = services;
    }) {
        doc.apply_adopted(e, dev);
    }
    Ok(())
}

/// "10.0.0.1/24", "10.0.0.1 255.255.255.0" -> the address with its prefix length
pub(crate) fn parse_addr(text: &str) -> Option<Prefix4> {
    let t = text.trim();
    if let Some((a, m)) = t.split_once(char::is_whitespace) {
        return Prefix4::from_mask(Ipv4::parse(a.trim())?, Ipv4::parse(m.trim())?);
    }
    if !t.contains('/') {
        return None; // a bare address would be a /32: say the length
    }
    Prefix4::parse(t).filter(|p| p.len > 0)
}

/// the interface's addresses become `text` (empty: none); its DHCP client ends
pub(crate) fn address(doc: &mut Doc, dev: u32, iface: &str, text: &str) -> Result<(), String> {
    let new = if text.trim().is_empty() {
        None
    } else {
        Some(parse_addr(text).ok_or_else(|| format!("„{}“ ist keine Adresse mit Präfix (10.0.0.1/24 oder 10.0.0.1 255.255.255.0).", text.trim()))?)
    };
    let d = doc.proj.device(dev).cloned().ok_or_else(|| format!("Das Gerät {} gibt es nicht.", dev))?;
    let mut acts = Vec::new();
    let mut services = d.services.clone();
    match dhcp_service(&d, iface) {
        // the client gives its lease back (that address goes with it)
        Some(i) => {
            services.remove(i);
            acts.push(Action::DhcpClient { dev, iface: String::from(iface), on: false });
        }
        None => {
            let now: Vec<Prefix4> = doc.sim.engine().ifaces(dev).into_iter().find(|f| jfn_model::ifname::same(&f.name, iface)).map(|f| f.addrs).unwrap_or_default();
            for p in now {
                acts.push(Action::IfAddr { dev, iface: String::from(iface), prefix: p, add: false });
            }
        }
    }
    if let Some(p) = new {
        acts.push(Action::IfAddr { dev, iface: String::from(iface), prefix: p, add: true });
    }
    let label = format!("Adresse von {} {} ändern", d.name, jfn_model::ifname::short(iface));
    apply(doc, dev, &label, &acts, Some(services))
}

/// the interface's DHCP client on (its static addresses go) or off
pub(crate) fn dhcp(doc: &mut Doc, dev: u32, iface: &str, on: bool) -> Result<(), String> {
    let d = doc.proj.device(dev).cloned().ok_or_else(|| format!("Das Gerät {} gibt es nicht.", dev))?;
    let mut services = d.services.clone();
    let have = dhcp_service(&d, iface);
    let mut acts = Vec::new();
    if on {
        if have.is_some() {
            return Ok(());
        }
        let now: Vec<Prefix4> = doc.sim.engine().ifaces(dev).into_iter().find(|f| jfn_model::ifname::same(&f.name, iface)).map(|f| f.addrs).unwrap_or_default();
        for p in now {
            acts.push(Action::IfAddr { dev, iface: String::from(iface), prefix: p, add: false });
        }
        acts.push(Action::DhcpClient { dev, iface: String::from(iface), on: true });
        let config = Value::object([("iface", Value::string(iface))]);
        services.push(ServiceDef { kind: String::from(DHCP_CLIENT), name: format!("{}@{}", DHCP_CLIENT, iface), disabled: false, config });
    } else {
        let Some(i) = have else { return Ok(()) };
        services.remove(i);
        acts.push(Action::DhcpClient { dev, iface: String::from(iface), on: false });
    }
    let label = format!("DHCP von {} {} {}", d.name, jfn_model::ifname::short(iface), if on { "einschalten" } else { "ausschalten" });
    apply(doc, dev, &label, &acts, Some(services))
}

/// an interface (with IPv4) or a port (a switch port) up or shut down
pub(crate) fn iface_up(doc: &mut Doc, dev: u32, name: &str, ip: bool, up: bool) -> Result<(), String> {
    let d = doc.proj.device(dev).cloned().ok_or_else(|| format!("Das Gerät {} gibt es nicht.", dev))?;
    let a = if ip { Action::IfUp { dev, iface: String::from(name), up } } else { Action::PortAdmin { dev, port: String::from(name), up } };
    let label = format!("{} {} {}", d.name, jfn_model::ifname::short(name), if up { "einschalten" } else { "abschalten" });
    apply(doc, dev, &label, &[a], None)
}

/// the default gateway of a host (empty: none)
pub(crate) fn gateway(doc: &mut Doc, dev: u32, text: &str) -> Result<(), String> {
    let new = if text.trim().is_empty() { None } else { Some(Ipv4::parse(text.trim()).ok_or_else(|| format!("„{}“ ist keine IPv4-Adresse.", text.trim()))?) };
    let d = doc.proj.device(dev).cloned().ok_or_else(|| format!("Das Gerät {} gibt es nicht.", dev))?;
    let any = Prefix4::new(Ipv4::from_u32(0), 0).ok_or_else(|| String::from("0.0.0.0/0"))?;
    let mut acts = Vec::new();
    for gw in default_gateways(doc.sim.engine(), dev) {
        acts.push(Action::Route { dev, prefix: any, via: Some(gw), iface: None, ad: 1, add: false });
    }
    if let Some(gw) = new {
        acts.push(Action::Route { dev, prefix: any, via: Some(gw), iface: None, ad: 1, add: true });
    }
    let label = format!("Gateway von {} ändern", d.name);
    apply(doc, dev, &label, &acts, None)
}

/// the configured default gateways of a device (static routes to 0.0.0.0/0)
pub(crate) fn default_gateways(e: &Engine, dev: u32) -> Vec<Ipv4> {
    let Some(st) = e.node(dev).and_then(|n| n.ip.as_ref()) else { return Vec::new() };
    st.statics.iter().filter(|r| r.prefix.len == 0).filter_map(|r| r.via).collect()
}
