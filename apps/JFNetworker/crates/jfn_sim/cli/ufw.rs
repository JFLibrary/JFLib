// ufw -- the small firewall of a Linux host on the shell: "ufw enable", "disable", "default", "allow",
// "deny", "reject", "delete", "status", "reset"; the words of the commands follow the original, the
// texts and the layout of status are our own.  It acts in fw.rs (HostFw), on that file's sessions
//
//   Engine::sh_ufw(ni, args)    the command
//   parse_rule(args)            the words of a rule as a HostRule (Err: what is wrong)
//   config(&HostFw)             the lines of the device's configuration (linux_config), "ufw enable" last
//
// A rule: "[out] PORT[/tcp|/udp]", "from NET", "from NET to any port PORT[/tcp|/udp]"; NET is an address
// with or without a length.  Rules count in the order they were made.

use alloc::format;
use alloc::string::String;

use netpkt::ipv4::{PROTO_TCP, PROTO_UDP};
use netpkt::{Ipv4, Prefix4};

use crate::action::Action;
use crate::fw::{FwAction, FwOp, HostFw, HostOp, HostRule};
use crate::kernel::Engine;

/// an action word as the shell takes it; allowed packets begin a session (fw.rs)
fn action_of(w: &str) -> Option<FwAction> {
    match w {
        "allow" => Some(FwAction::Inspect),
        "deny" => Some(FwAction::Drop),
        "reject" => Some(FwAction::Reject),
        _ => None,
    }
}

fn action_word(a: FwAction) -> &'static str {
    match a {
        FwAction::Inspect | FwAction::Pass => "allow",
        FwAction::Drop => "deny",
        FwAction::Reject => "reject",
    }
}

/// "80", "80/tcp", "53/udp"
fn port_of(w: &str) -> Result<(u16, Option<u8>), String> {
    let (p, proto) = match w.split_once('/') {
        Some((p, "tcp")) => (p, Some(PROTO_TCP)),
        Some((p, "udp")) => (p, Some(PROTO_UDP)),
        Some(_) => return Err(format!("ufw: {}: only /tcp and /udp\n", w)),
        None => (w, None),
    };
    match p.parse::<u16>() {
        Ok(n) if n > 0 => Ok((n, proto)),
        _ => Err(format!("ufw: {}: no port\n", w)),
    }
}

/// the words of a rule after its action
pub fn parse_rule(action: FwAction, args: &[&str]) -> Result<HostRule, String> {
    let (out, rest) = match args.first() {
        Some(&"out") => (true, &args[1..]),
        Some(&"in") => (false, &args[1..]),
        _ => (false, args),
    };
    let mut r = HostRule { action, out, port: None, proto: None, from: None };
    match rest {
        [p] => {
            let (n, proto) = port_of(p)?;
            r.port = Some(n);
            r.proto = proto;
        }
        ["from", net] | ["from", net, "to", "any", "port", ..] => {
            r.from = Some(net_of(net)?);
            if let ["from", _, "to", "any", "port", p] = rest {
                let (n, proto) = port_of(p)?;
                r.port = Some(n);
                r.proto = proto;
            } else if rest.len() != 2 {
                return Err(String::from("ufw: a rule: PORT[/tcp|/udp], or from NET [to any port PORT]\n"));
            }
        }
        _ => return Err(String::from("ufw: a rule: PORT[/tcp|/udp], or from NET [to any port PORT]\n")),
    }
    Ok(r)
}

fn net_of(w: &str) -> Result<Prefix4, String> {
    if w.contains('/') {
        return Prefix4::parse(w).map(|p| p.trunc()).ok_or_else(|| format!("ufw: {}: no network\n", w));
    }
    Ipv4::parse(w).map(|addr| Prefix4 { addr, len: 32 }).ok_or_else(|| format!("ufw: {}: no address\n", w))
}

/// the lines of the device's configuration
pub fn config(h: &HostFw) -> String {
    let d = HostFw::default();
    let mut s = String::new();
    if h.default_in != d.default_in {
        s.push_str(&format!("ufw default {} incoming\n", action_word(h.default_in)));
    }
    if h.default_out != d.default_out {
        s.push_str(&format!("ufw default {} outgoing\n", action_word(h.default_out)));
    }
    for r in &h.rules {
        s.push_str(&format!("ufw {} {}\n", action_word(r.action), r.words()));
    }
    if h.enabled {
        s.push_str("ufw enable\n");
    }
    s
}

impl Engine {
    fn ufw_do(&mut self, ni: usize, op: HostOp, said: &str) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let _ = self.apply(Action::Fw { dev, op: FwOp::Host(op) });
        self.out(ni, said);
        Ok(())
    }

    /// the command on the shell
    pub(crate) fn sh_ufw(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        match args {
            ["enable"] => self.ufw_do(ni, HostOp::Enable(true), "The firewall of this host is on.\n"),
            ["disable"] => self.ufw_do(ni, HostOp::Enable(false), "The firewall of this host is off.\n"),
            ["reset"] => self.ufw_do(ni, HostOp::Reset, "The firewall of this host is off, its rules are gone.\n"),
            ["status", ..] => {
                let Some(h) = self.firewall(dev).map(|f| f.host.clone()) else {
                    return Ok(());
                };
                let mut s = format!("firewall of this host: {}\n", if h.enabled { "on" } else { "off" });
                s.push_str(&format!("without a rule: incoming {}, outgoing {}\n", action_word(h.default_in), action_word(h.default_out)));
                for (i, r) in h.rules.iter().enumerate() {
                    s.push_str(&format!("  {} {} {}\n", i + 1, action_word(r.action), r.words()));
                }
                if h.enabled {
                    let n = self.fw_sessions(dev).len();
                    s.push_str(&format!("sessions now: {}\n", n));
                }
                self.out(ni, &s);
                Ok(())
            }
            ["default", word, rest @ ..] => {
                let Some(action) = action_of(word) else {
                    return self.sh_err(ni, "ufw: default: allow, deny or reject\n");
                };
                let out = match rest {
                    [] | ["incoming"] => false,
                    ["outgoing"] => true,
                    _ => return self.sh_err(ni, "ufw: default: incoming or outgoing\n"),
                };
                self.ufw_do(ni, HostOp::Default { out, action }, "The default is set.\n")
            }
            ["delete", word, rest @ ..] => {
                let Some(action) = action_of(word) else {
                    return self.sh_err(ni, "ufw: delete: allow, deny or reject and the rule's words\n");
                };
                match parse_rule(action, rest) {
                    Ok(r) => self.ufw_do(ni, HostOp::Remove(r), "The rule is gone.\n"),
                    Err(e) => self.sh_err(ni, &e),
                }
            }
            [word, rest @ ..] if action_of(word).is_some() => {
                let action = action_of(word).unwrap_or(FwAction::Drop);
                match parse_rule(action, rest) {
                    Ok(r) => self.ufw_do(ni, HostOp::Add(r), "The rule is added.\n"),
                    Err(e) => self.sh_err(ni, &e),
                }
            }
            _ => self.sh_err(ni, "ufw: enable, disable, status, reset, default, allow, deny, reject, delete\n"),
        }
    }
}
