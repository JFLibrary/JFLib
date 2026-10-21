// scripts -- the shell's script command (our own tool, our own words): the device's scripts
// (scriptsvc.rs)
//
//   script [list]                     every script: state, callbacks, errors, CPU time, ports
//   script status [NAME]              one script: state, CPU time and what one instruction
//                                     costs, the queue, the limits, what it has open, its own
//                                     metrics, the last error
//   script start|stop|restart NAME    starts it, stops it, or both (a start begins with a
//                                     fresh program; the disk and the numbers stay)
//   script errors [NAME]              the last errors: time, line and column, the calls
//   script output [NAME]              what it printed last (print)
//
// Without NAME a command takes the device's first script.  Times are virtual.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::lab::{row, table};
use crate::disk::size_text;
use crate::kernel::Engine;
use crate::scriptsvc::{ScriptErr, ScriptService, ScriptState, ScriptView};
use crate::trace::fmt_ns;

const HELP: &str = "script [list]                   zeigt die Skripte des Geräts: Zustand, Rückrufe, Fehler, CPU-Zeit\nscript status [NAME]            ein Skript: Zustand, Rechenzeit, Warteschlange, Grenzen, Offenes, Metriken\nscript start|stop|restart NAME  startet ein Skript, hält es an oder startet es neu\nscript errors [NAME]            die letzten Fehler mit Zeit, Zeile und Spalte\nscript output [NAME]            was das Skript zuletzt mit print ausgab\n";

fn state_word(v: &ScriptView) -> &'static str {
    match v.state {
        ScriptState::Running if v.busy => "rechnet",
        ScriptState::Running => "läuft",
        ScriptState::Stopped => "gestoppt",
        ScriptState::Failed => "Fehler",
    }
}

fn ports_text(v: &ScriptView) -> String {
    let p: Vec<String> = v.ports.iter().map(|(k, n)| format!("{} {}", k, n)).collect();
    if p.is_empty() { String::from("nichts") } else { p.join(", ") }
}

/// "5 ns", "2,5 ns", "0,125 ns" of picoseconds
fn per_op(ps: u64) -> String {
    let (n, mut f) = (ps / 1000, format!("{:03}", ps % 1000));
    while f.ends_with('0') {
        f.pop();
    }
    if f.is_empty() { format!("{} ns", n) } else { format!("{},{} ns", n, f) }
}

/// "0,512 s  Zeile 3, Spalte 5 (data): Division durch 0"
fn err_line(e: &ScriptErr) -> String {
    if e.line > 0 {
        format!("{}  Zeile {}, Spalte {} ({}): {}", fmt_ns(e.at as i64), e.line, e.col, e.during, e.msg)
    } else {
        format!("{}  ({}): {}", fmt_ns(e.at as i64), e.during, e.msg)
    }
}

impl Engine {
    fn script_err(&mut self, ni: usize, text: &str) -> Result<(), String> {
        self.out(ni, text);
        Err(String::from(text.lines().next().unwrap_or("")))
    }

    /// the script a command means: NAME, else the device's first one
    fn script_pick(&self, ni: usize, name: Option<&str>) -> Option<(String, ScriptView)> {
        let all = self.scripts(self.dev_of(ni));
        match name {
            Some(n) => all.into_iter().find(|s| s.0.eq_ignore_ascii_case(n)),
            None => all.into_iter().next(),
        }
    }

    /// "script ...": the device's scripts
    pub(crate) fn sh_script(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let (cmd, name) = match args {
            [] => ("list", None),
            ["-h" | "--help" | "help"] => {
                self.out(ni, HELP);
                return Ok(());
            }
            [c] => (*c, None),
            [c, n] => (*c, Some(*n)),
            _ => return self.script_err(ni, &format!("script: {} kenne ich nicht\n{}", args.join(" "), HELP)),
        };
        if !matches!(cmd, "list" | "status" | "errors" | "output" | "start" | "stop" | "restart") || (cmd == "list" && name.is_some()) {
            return self.script_err(ni, &format!("script: {} kenne ich nicht\n{}", args.join(" "), HELP));
        }
        if matches!(cmd, "start" | "stop" | "restart") && name.is_none() {
            return self.script_err(ni, &format!("script {}: welches Skript? (script {} NAME)\n", cmd, cmd));
        }
        let Some((n, v)) = self.script_pick(ni, name) else {
            return match name {
                Some(x) => self.script_err(ni, &format!("script: {} ist kein Skript dieses Geräts\n", x)),
                None => self.script_err(ni, "script: auf diesem Gerät läuft kein Skript\n"),
            };
        };
        let svc = self.service::<ScriptService>(dev, &n);
        let errors = svc.map(|s| s.errors()).unwrap_or_default();
        let text = match cmd {
            "list" => {
                let mut rows = alloc::vec![row(&["Skript", "Zustand", "Rückrufe", "Fehler", "CPU-Zeit", "offen"])];
                for (n, v) in self.scripts(dev) {
                    rows.push(alloc::vec![n, String::from(state_word(&v)), format!("{}", v.calls), format!("{}", v.errors), fmt_ns(v.busy_ns as i64), ports_text(&v)]);
                }
                table(&rows)
            }
            "status" => {
                let mut s = format!("Skript {}: {} seit {}, {} Zeilen\n", n, state_word(&v), fmt_ns(v.since as i64), v.lines);
                s.push_str(&format!("Rückrufe {}, Fehler {}, Anweisungen {}, CPU-Zeit {} ({} je Anweisung)\n", v.calls, v.errors, v.fuel, fmt_ns(v.busy_ns as i64), per_op(v.cfg.ps_per_op)));
                s.push_str(&format!("Warteschlange {} (höchstens {}, Platz für {}), verworfen {}, abgewiesen {}\n", v.queued, v.max_queued, v.cfg.queue, v.dropped, v.rejected));
                let fuel = if v.cfg.limits.fuel == u64::MAX { String::from("unbegrenzt") } else { format!("{} Anweisungen", v.cfg.limits.fuel) };
                s.push_str(&format!("Grenzen je Rückruf: {}, Tiefe {}, Stapel {}; Speicher {} von {}\n", fuel, v.cfg.limits.depth, v.cfg.limits.stack, size_text(v.heap), size_text(v.cfg.limits.heap)));
                s.push_str(&format!("Offen: {}; Verbindungen {}, Timer {}\n", ports_text(&v), v.conns, v.timers));
                if !v.metrics.is_empty() {
                    let m: Vec<String> = v.metrics.iter().map(|(k, x)| format!("{} {}", k, x)).collect();
                    s.push_str(&format!("Metriken: {}\n", m.join(", ")));
                }
                if let Some(e) = errors.last() {
                    s.push_str(&format!("Letzter Fehler: {}\n", err_line(e)));
                }
                s
            }
            "errors" => {
                if errors.is_empty() {
                    format!("{}: keine Fehler\n", n)
                } else {
                    let mut s = String::new();
                    for e in &errors {
                        s.push_str(&err_line(e));
                        s.push('\n');
                        for t in &e.trace {
                            let f = match t.func.as_str() {
                                "" => String::from("im Hauptprogramm"),
                                "fn" => String::from("in einer Funktion ohne Namen"),
                                f => format!("in {}", f),
                            };
                            s.push_str(&format!("           {} ({}:{})\n", f, t.line, t.col));
                        }
                    }
                    s
                }
            }
            "output" => {
                let out = svc.map(|s| String::from(s.output())).unwrap_or_default();
                if out.is_empty() {
                    format!("{}: noch keine Ausgabe\n", n)
                } else if out.ends_with('\n') {
                    out
                } else {
                    format!("{}\n", out)
                }
            }
            _ => {
                let run = if cmd == "start" { v.state == ScriptState::Running } else { false };
                if cmd != "start" && let Err(e) = self.set_service_running(dev, &n, false) {
                    return self.script_err(ni, &format!("script {}: {}\n", cmd, e.msg().text()));
                }
                if cmd != "stop" && !run && let Err(e) = self.set_service_running(dev, &n, true) {
                    return self.script_err(ni, &format!("script {}: {}\n", cmd, e.msg().text()));
                }
                String::new()
            }
        };
        self.out(ni, &text);
        Ok(())
    }
}
