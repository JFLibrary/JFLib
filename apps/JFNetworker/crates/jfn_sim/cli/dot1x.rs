// dot1x -- 802.1X at the command lines: the authenticator's commands of IOS-like switches, the
// lines of their running configuration, show and debug in our own layout; wpa_supplicant and
// wpa_cli on the shell.  Help texts, tables and messages are our own words, command words and
// configuration lines follow the originals
//
//   build_dot1x(tree, modes)            the commands in an IOS tree (ios.rs; switches, no APs)
//   Engine::ios_dot1x(ni, p)            the global and EXEC commands
//   Engine::ios_dot1x_if(ni, if, p)     a command of interface mode for one port
//   Engine::ios_dot1x_config(ni)        the global lines of the running configuration
//   Engine::ios_dot1x_if_config(ni, p)  the lines of an interface
//   Engine::dot1x_debug(note)           a decision as a debug line (debug dot1x)
//   Engine::sh_wpa_supplicant / sh_wpa_cli   the shell's commands
//
// IOS-like:
//   dot1x system-auth-control                    802.1X on for the device
//   aaa authentication dot1x default METHOD...   group radius | group NAME | local
//   authentication port-control auto | force-authorized | force-unauthorized    (interface)
//   dot1x pae authenticator, dot1x timeout tx-period | quiet-period | reauth-period |
//   supp-timeout N, dot1x max-req N, authentication periodic, authentication timer reauthenticate N
//   show dot1x [all | interface IF], show authentication sessions [interface IF]
//   dot1x re-authenticate interface IF, clear authentication sessions
//   debug dot1x [all], undebug dot1x [all]
// Linux:
//   wpa_supplicant [-B] -i IF -c FILE [-D DRIVER]   runs the supplicant with that file
//   wpa_cli status | list_networks | reconfigure | logon | logoff | reauthenticate [-i IF]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_NO};

use super::ios::K;
use crate::dot1x::{PaeState, PortCfg, PortCtl, SesKey};
use crate::kernel::{Engine, SEC};
use crate::supp::{KeyMgmt, SuppState, CONF};
use crate::trace::Note;

/// the commands of 802.1X; `m`: user, privileged, global configuration and interface mode
pub(crate) fn build_dot1x(t: &mut Tree<K>, m: [ModeId; 4]) {
    let [user, prv, conf, ifm] = m;
    for x in [user, prv] {
        t.add(x, "show dot1x", "Port access control: the device's state", Some(K::ShowDot1x), 0);
        t.add(x, "show dot1x all", "The state and every port", Some(K::ShowDot1x), 0);
        t.add(x, "show dot1x interface", "One port", None, 0);
        t.add(x, "show dot1x interface NAME1=if", "", Some(K::ShowDot1x), 0);
        t.add(x, "show authentication", "Who authenticated on the ports", None, 0);
        t.add(x, "show authentication sessions", "The running sessions", Some(K::ShowAuthSes), 0);
        t.add(x, "show authentication sessions interface", "One port", None, 0);
        t.add(x, "show authentication sessions interface NAME1=if", "", Some(K::ShowAuthSes), 0);
    }
    t.add(prv, "dot1x", "Port access control now", None, 0);
    t.add(prv, "dot1x re-authenticate", "Ask a port's supplicant again", None, 0);
    t.add(prv, "dot1x re-authenticate interface", "One port", None, 0);
    t.add(prv, "dot1x re-authenticate interface NAME1=if", "", Some(K::Dot1xNow), 0);
    t.add(prv, "clear authentication", "Authentication state", None, 0);
    t.add(prv, "clear authentication sessions", "Every port authenticates again", Some(K::Dot1xNow), 0);
    t.add(prv, "debug dot1x", "What port access control does while it happens", Some(K::Debug), 0);
    t.add(prv, "debug dot1x all", "Everything of it", Some(K::Debug), 0);
    t.add(prv, "undebug dot1x", "Stop its debug output", Some(K::Undebug), 0);
    t.add(prv, "undebug dot1x all", "Everything of it", Some(K::Undebug), 0);

    t.add(conf, "aaa authentication dot1x", "Who may use a port", None, 0);
    t.add(conf, "aaa authentication dot1x default", "The default list", None, 0);
    t.add(conf, "aaa authentication dot1x default LINE=methods", "Methods in order (group radius | group NAME | local)", Some(K::AaaAuthDot1x), 0);
    t.add_no(conf, "aaa authentication dot1x default", K::AaaAuthDot1x);
    t.add(conf, "dot1x", "Port access control of the device", None, F_NO);
    t.add(conf, "dot1x system-auth-control", "Turn port access control on", Some(K::Dot1xSystem), 0);

    t.add(ifm, "authentication", "Port access control of this port", None, F_NO);
    t.add(ifm, "authentication port-control", "Who decides whether the port is open", None, 0);
    t.add(ifm, "authentication port-control auto=ctl", "A supplicant and the server decide", Some(K::Dot1xIf), 0);
    t.add(ifm, "authentication port-control force-authorized=ctl", "Open without asking", Some(K::Dot1xIf), 0);
    t.add(ifm, "authentication port-control force-unauthorized=ctl", "Closed whatever comes", Some(K::Dot1xIf), 0);
    t.add_no(ifm, "authentication port-control", K::Dot1xIf);
    t.add(ifm, "authentication periodic", "Authenticate again from time to time", Some(K::Dot1xIf), 0);
    t.add(ifm, "authentication timer", "Timers of this port", None, 0);
    t.add(ifm, "authentication timer reauthenticate", "Seconds until it authenticates again", None, 0);
    t.add(ifm, "authentication timer reauthenticate <1-65535>=reauth", "Seconds", Some(K::Dot1xIf), 0);
    t.add_no(ifm, "authentication timer reauthenticate", K::Dot1xIf);
    t.add(ifm, "dot1x", "802.1X settings of this port", None, F_NO);
    t.add(ifm, "dot1x pae", "The role of this port", None, 0);
    t.add(ifm, "dot1x pae authenticator", "It asks the supplicant at its far end", Some(K::Dot1xIf), 0);
    t.add(ifm, "dot1x timeout", "How long the port waits", None, 0);
    for (kw, h) in [("tx-period", "Between identity requests"), ("quiet-period", "After a refusal"), ("reauth-period", "Until it authenticates again"), ("supp-timeout", "For the supplicant's answer")] {
        t.add(ifm, &format!("dot1x timeout {}", kw), h, None, 0);
        t.add(ifm, &format!("dot1x timeout {} <1-65535>=secs", kw), "Seconds", Some(K::Dot1xIf), 0);
        t.add_no(ifm, &format!("dot1x timeout {}", kw), K::Dot1xIf);
    }
    t.add(ifm, "dot1x max-req", "How often a request goes out again", None, 0);
    t.add(ifm, "dot1x max-req <1-10>=max", "Times", Some(K::Dot1xIf), 0);
    t.add_no(ifm, "dot1x max-req", K::Dot1xIf);
}

/// "00:01:23"
fn hms(ns: u64) -> String {
    let s = ns / SEC;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

impl Engine {
    fn dot1x_port_of(&self, ni: usize, name: &str) -> Option<u16> {
        self.nodes.get(ni)?.as_ref()?.port_by_name(name)
    }

    /// the global and EXEC commands
    pub(crate) fn ios_dot1x(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        match p.cmd {
            K::Dot1xSystem => {
                if let Some(c) = self.cdev_mut(ni) {
                    c.cfg.dot1x.system = !p.no;
                }
                self.dot1x_kick(ni);
                Ok(())
            }
            K::AaaAuthDot1x => {
                let ms = if p.no {
                    Vec::new()
                } else {
                    match super::aaa::parse_methods(p.text("methods").unwrap_or("")) {
                        Ok(m) => m,
                        Err(e) => return self.ios_err(ni, &e),
                    }
                };
                if ms.iter().any(|m| !matches!(m, crate::aaa::Method::GroupRadius | crate::aaa::Method::Group(_) | crate::aaa::Method::Local)) {
                    return self.ios_err(ni, "% 802.1X takes group radius, group NAME or local\n");
                }
                if let Some(c) = self.cdev_mut(ni) {
                    c.cfg.dot1x.methods = ms;
                }
                Ok(())
            }
            K::ShowDot1x => {
                let t = match p.text("if") {
                    Some(f) => {
                        let Some(port) = self.dot1x_port_of(ni, f) else {
                            return self.ios_err(ni, "% No such port\n");
                        };
                        self.show_dot1x_port(ni, port)
                    }
                    None => self.show_dot1x(ni, p.has("all")),
                };
                self.out(ni, &t);
                Ok(())
            }
            K::ShowAuthSes => {
                let only = match p.text("if") {
                    Some(f) => match self.dot1x_port_of(ni, f) {
                        Some(port) => Some(port),
                        None => return self.ios_err(ni, "% No such port\n"),
                    },
                    None => None,
                };
                let t = self.show_auth_sessions(ni, only);
                self.out(ni, &t);
                Ok(())
            }
            K::Dot1xNow => {
                let port = match p.text("if") {
                    Some(f) => match self.dot1x_port_of(ni, f) {
                        Some(port) => Some(port),
                        None => return self.ios_err(ni, "% No such port\n"),
                    },
                    None => None,
                };
                self.dot1x_reauth(ni, port);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// a command of interface mode for one port
    pub(crate) fn ios_dot1x_if(&mut self, ni: usize, f: &str, p: &Parsed<K>, line: &str) -> Result<(), String> {
        let Some(port) = self.dot1x_port_of(ni, f) else {
            return self.ios_err(ni, "% Only switch ports take 802.1X\n");
        };
        let words: Vec<&str> = line.split_whitespace().filter(|w| *w != "no").collect();
        let member = self.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.bridge.as_ref()).and_then(|b| b.ports.get(port as usize)).map(|b| (b.member, b.admin));
        let Some((true, admin)) = member else {
            return self.ios_err(ni, "% Only switch ports take 802.1X\n");
        };
        if admin == crate::bridge::SwMode::Trunk && p.text("ctl") == Some("auto") {
            return self.ios_err(ni, "% Port access control needs an access port, not a trunk\n");
        }
        let Some(c) = self.cdev_mut(ni) else { return Ok(()) };
        let x = c.cfg.dot1x.ports.entry(port).or_default();
        let d = PortCfg::default();
        match words.as_slice() {
            ["authentication", "port-control", ..] | ["auth", ..] if p.has("ctl") || p.no => {
                x.ctl = match p.text("ctl") {
                    Some("auto") => PortCtl::Auto,
                    Some("force-unauthorized") => PortCtl::ForceUnauth,
                    _ => PortCtl::ForceAuth,
                };
            }
            ["authentication", "periodic", ..] => x.periodic = !p.no,
            ["authentication", "timer", ..] => x.reauth_period = p.num("reauth").unwrap_or(0) as u32,
            ["dot1x", "pae", ..] => x.pae = Some(!p.no),
            ["dot1x", "timeout", "tx-period", ..] => x.tx_period = p.num("secs").map_or(d.tx_period, |v| v as u32),
            ["dot1x", "timeout", "quiet-period", ..] => x.quiet_period = p.num("secs").map_or(d.quiet_period, |v| v as u32),
            ["dot1x", "timeout", "reauth-period", ..] => x.reauth_period = p.num("secs").unwrap_or(0) as u32,
            ["dot1x", "timeout", "supp-timeout", ..] => x.supp_timeout = p.num("secs").map_or(d.supp_timeout, |v| v as u32),
            ["dot1x", "max-req", ..] => x.max_req = p.num("max").map_or(d.max_req, |v| v as u8),
            _ => {}
        }
        // a port at its defaults is not kept
        if *x == d {
            c.cfg.dot1x.ports.remove(&port);
        }
        self.dot1x_kick(ni);
        Ok(())
    }

    /// the global lines of the running configuration
    pub(crate) fn ios_dot1x_config(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else { return String::new() };
        let mut s = String::new();
        if !c.cfg.dot1x.methods.is_empty() {
            let words: Vec<String> = c.cfg.dot1x.methods.iter().map(|m| m.words()).collect();
            s.push_str(&format!("aaa authentication dot1x default {}\n", words.join(" ")));
        }
        if c.cfg.dot1x.system {
            s.push_str("dot1x system-auth-control\n");
        }
        if !s.is_empty() {
            s.push_str("!\n");
        }
        s
    }

    /// the lines of an interface
    pub(crate) fn ios_dot1x_if_config(&self, ni: usize, port: u16) -> String {
        let Some(x) = self.cdev(ni).and_then(|c| c.cfg.dot1x.ports.get(&port)) else { return String::new() };
        let d = PortCfg::default();
        let mut s = String::new();
        if x.ctl != d.ctl {
            s.push_str(&format!(" authentication port-control {}\n", x.ctl.word()));
        }
        if x.periodic {
            s.push_str(" authentication periodic\n");
        }
        match x.pae {
            Some(true) => s.push_str(" dot1x pae authenticator\n"),
            Some(false) => s.push_str(" no dot1x pae authenticator\n"),
            None => {}
        }
        for (kw, v, def) in [("tx-period", x.tx_period, d.tx_period), ("quiet-period", x.quiet_period, d.quiet_period), ("supp-timeout", x.supp_timeout, d.supp_timeout), ("reauth-period", x.reauth_period, d.reauth_period)] {
            if v != def {
                s.push_str(&format!(" dot1x timeout {} {}\n", kw, v));
            }
        }
        if x.max_req != d.max_req {
            s.push_str(&format!(" dot1x max-req {}\n", x.max_req));
        }
        s
    }

    /// "show dot1x [all]"
    fn show_dot1x(&self, ni: usize, all: bool) -> String {
        let Some(c) = self.cdev(ni) else { return String::new() };
        let cfg = &c.cfg.dot1x;
        let mut s = format!("Port access control: {}\nProtocol version: 2 (802.1X-2004)\n", if cfg.system { "on" } else { "off" });
        if cfg.methods.is_empty() {
            s.push_str("Who decides: no method list (aaa authentication dot1x)\n");
        } else {
            let w: Vec<String> = cfg.methods.iter().map(|m| m.words()).collect();
            s.push_str(&format!("Who decides: {}\n", w.join(", ")));
        }
        if all {
            let ports: Vec<u16> = cfg.ports.keys().copied().collect();
            for p in ports {
                s.push('\n');
                s.push_str(&self.show_dot1x_port(ni, p));
            }
        }
        s
    }

    /// "show dot1x interface IF"
    fn show_dot1x_port(&self, ni: usize, port: u16) -> String {
        let Some(n) = self.nodes.get(ni).and_then(|n| n.as_ref()) else { return String::new() };
        let name = String::from(n.port_name(port));
        let d = PortCfg::default();
        let x = n.cli.cfg.dot1x.ports.get(&port).unwrap_or(&d);
        let closed = n.bridge.as_ref().and_then(|b| b.ports.get(port as usize)).is_some_and(|b| b.closed);
        let dev = n.dev;
        let ses = self.dot1x_sessions(dev).into_iter().find(|s| s.key == SesKey::Port(port));
        let mut s = format!("{}: {}\n", name, if closed { "closed, only EAPOL passes" } else { "open" });
        s.push_str(&format!("  Port control      {}\n", x.ctl.word()));
        s.push_str(&format!("  Role              {}\n", if x.pae == Some(false) { "none" } else { "authenticator" }));
        s.push_str(&format!("  Waits (seconds)   between requests {}, after a refusal {}, for an answer {}\n", x.tx_period, x.quiet_period, x.supp_timeout));
        s.push_str(&format!("  Requests again    up to {} times\n", x.max_req));
        if x.periodic {
            s.push_str(&match x.reauth_period {
                0 => String::from("  Asks again        as the server says (Session-Timeout)\n"),
                r => format!("  Asks again        every {} s\n", r),
            });
        } else {
            s.push_str("  Asks again        no\n");
        }
        match ses {
            Some(v) => {
                s.push_str(&format!("  State             {}\n", v.state.word()));
                if !v.user.is_empty() {
                    s.push_str(&format!("  User              {}\n", v.user));
                }
                if let Some(m) = v.mac {
                    s.push_str(&format!("  Supplicant        {}\n", m.cisco()));
                }
                if !v.method.is_empty() {
                    s.push_str(&format!("  Method            {}\n", v.method));
                }
                if let Some(vl) = v.vlan {
                    s.push_str(&format!("  VLAN from server  {}\n", vl));
                }
            }
            None => s.push_str("  State             no session\n"),
        }
        s
    }

    /// "show authentication sessions [interface IF]"
    fn show_auth_sessions(&self, ni: usize, only: Option<u16>) -> String {
        let dev = self.dev_of(ni);
        let now = self.now;
        let rows: Vec<_> = self.dot1x_sessions(dev).into_iter().filter(|s| only.is_none_or(|p| s.key == SesKey::Port(p))).collect();
        if rows.is_empty() {
            return String::from("No sessions\n");
        }
        let mut s = format!("{:<12} {:<16} {:<16} {:<6} {:<6} {:<15} {}\n", "Port", "User", "Client", "Method", "VLAN", "State", "For");
        for v in rows {
            let client = v.mac.map(|m| m.cisco()).unwrap_or_else(|| String::from("-"));
            let vlan = v.vlan.map(|x| format!("{}", x)).unwrap_or_else(|| String::from("-"));
            let state = match v.state {
                PaeState::Authenticating if v.open => "REAUTH",
                st => st.word(),
            };
            let user = if v.user.is_empty() { "-" } else { v.user.as_str() };
            let method = if v.method.is_empty() { "-" } else { v.method.as_str() };
            s.push_str(&format!("{:<12} {:<16} {:<16} {:<6} {:<6} {:<15} {}\n", v.name, user, client, method, vlan, state, hms(now.saturating_sub(v.since))));
        }
        s
    }

    /// a decision as a debug line
    pub(crate) fn dot1x_debug(&self, note: &Note) -> String {
        let Note::Dot1x(n) = note else { return String::new() };
        format!("dot1x: {}\n", n.message().text())
    }

    // ---------------------------------------------------------------- the shell

    /// "wpa_supplicant ..."
    pub(crate) fn sh_wpa_supplicant(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut file = None;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-c" => {
                    file = args.get(i + 1).copied();
                    i += 1;
                }
                "-i" | "-D" => i += 1,
                "-B" => {}
                w if w.starts_with('-') => return self.sh_err(ni, &format!("wpa_supplicant: option {} is not known here\n", w)),
                _ => {}
            }
            i += 1;
        }
        let Some(file) = file else {
            return self.sh_err(ni, "Usage: wpa_supplicant [-B] -i IF -c FILE [-D DRIVER]\n");
        };
        let path = self.sh_abs(ni, file);
        if !self.store(ni).is_some_and(|s| s.exists(&path)) {
            return self.sh_err(ni, &format!("wpa_supplicant: {}: no such file\n", file));
        }
        self.supp_reconfigure(ni, Some(&path));
        self.out(ni, "The supplicant runs with this file.\n");
        Ok(())
    }

    /// "wpa_cli ..."
    pub(crate) fn sh_wpa_cli(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut rest: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if matches!(args[i], "-i" | "-p") {
                i += 1;
            } else {
                rest.push(args[i]);
            }
            i += 1;
        }
        let dev = self.dev_of(ni);
        match rest.as_slice() {
            ["status"] | [] => {
                let rows = self.supp_views(dev);
                let mut s = String::new();
                if self.os(ni).and_then(|o| o.svc_id(crate::supp::SVC)).is_none() {
                    s.push_str("The supplicant does not run.\n");
                } else if rows.is_empty() {
                    s.push_str("The supplicant runs; no port has anything to do.\n");
                }
                let now = self.now;
                for v in rows {
                    let what = match v.state {
                        SuppState::Off => String::from("off"),
                        SuppState::Connecting => String::from("looking for an authenticator"),
                        SuppState::NoPeer => String::from("nobody answers, the port runs without 802.1X"),
                        SuppState::Authenticating => format!("logging in as {}", v.user),
                        SuppState::Authenticated => format!("logged in as {} with {} for {}", v.user, v.method, hms(now.saturating_sub(v.since))),
                        SuppState::Held => format!("refused as {}, waiting", v.user),
                        SuppState::NoCreds => String::from("no identity to log in with"),
                    };
                    s.push_str(&format!("{}: {}\n", v.name, what));
                }
                self.out(ni, &s);
                Ok(())
            }
            ["list_networks"] => {
                let blocks = self.supp_conf(dev);
                let mut s = format!("{:<3} {:<20} {:<10} {:<6} {}\n", "#", "Network", "Key mgmt", "EAP", "Identity");
                for (k, b) in blocks.iter().enumerate() {
                    let km = match &b.key_mgmt {
                        KeyMgmt::Ieee8021x => "IEEE8021X",
                        KeyMgmt::WpaEap => "WPA-EAP",
                        KeyMgmt::Other(o) => o.as_str(),
                    };
                    let name = if b.ssid.is_empty() { "(wired)" } else { b.ssid.as_str() };
                    s.push_str(&format!("{:<3} {:<20} {:<10} {:<6} {}\n", k, name, km, b.eap.map(|m| m.word()).unwrap_or("-"), b.identity));
                }
                self.out(ni, &s);
                Ok(())
            }
            ["reconfigure"] => {
                if !self.store(ni).is_some_and(|s| s.exists(CONF)) && self.os(ni).and_then(|o| o.svc_id(crate::supp::SVC)).is_none() {
                    return self.sh_err(ni, &format!("wpa_cli: {} does not exist\n", CONF));
                }
                self.supp_reconfigure(ni, None);
                self.out(ni, "OK\n");
                Ok(())
            }
            ["logon"] | ["reauthenticate"] | ["logoff"] => {
                if self.supp_command(ni, rest[0]) {
                    self.out(ni, "OK\n");
                    Ok(())
                } else {
                    self.sh_err(ni, "wpa_cli: the supplicant does not run\n")
                }
            }
            ["help"] | ["--help"] | ["-h"] => {
                self.out(ni, "Usage: wpa_cli [-i IF] status | list_networks | reconfigure | logon | logoff | reauthenticate\n");
                Ok(())
            }
            [w, ..] => self.sh_err(ni, &format!("wpa_cli: {} is not known here; try \"wpa_cli help\"\n", w)),
        }
    }
}
