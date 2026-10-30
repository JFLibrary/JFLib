// debugger -- `jfs --debug FILE [ARGS ...]`: a small debugger on the console, its commands on
// standard input (the VM's debug interface, lib/script/debug.rs)
//
//   break FILE:LINE [if COND] / break LINE [if COND]  (b)   a breakpoint; LINE alone is in the
//                      script, FILE is a path from the working directory or the script's
//                      directory; COND a jfs expression
//   delete N (d)       remove breakpoint N; breaks: the list
//   run (r)            start the program (before it only breakpoints can be set)
//   continue (c)       go on to the next stop
//   step (s) / next (n) / finish (f)   the next statement, into calls / over them / back to
//                      the caller
//   print EXPR (p)     a read-only expression in the innermost call
//   bt                 the calls, innermost first
//   locals             the variables of the innermost call (with those its closure captured)
//   quit (q)           end the program (exit code 0); the end of the input is quit too
//   help (h)
//
// Every stop shows "Halt in FILE:LINE (Grund)" and that line of the source.  The prompt is
// "(jfs) ".  Debugger::new / prelude (the commands before run) / attach (the breakpoints into
// the VM) / stopped (a Status::Paused: the commands until the program goes on) / ended.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use script::{Pause, PauseReason, Step, Vm};

use crate::host::Console;
use crate::{err, out};

/// how the program goes on after a stop
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Go {
    On,
    Quit,
}

const HELP: &str = concat!(
    "Befehle:\n",
    "  break DATEI:ZEILE [if BEDINGUNG], break ZEILE   (b)  ein Haltepunkt\n",
    "  delete N (d)          Haltepunkt N entfernen; breaks: alle zeigen\n",
    "  run (r)               das Programm starten\n",
    "  continue (c)          weiter bis zum nächsten Halt\n",
    "  step (s)              eine Anweisung weiter, auch in Aufrufe hinein\n",
    "  next (n)              eine Anweisung weiter, über Aufrufe hinweg\n",
    "  finish (f)            weiter bis zurück beim Aufrufer\n",
    "  print AUSDRUCK (p)    einen Ausdruck im innersten Aufruf auswerten\n",
    "  bt                    die Aufrufe, der innerste zuerst\n",
    "  locals                die Variablen des innersten Aufrufs\n",
    "  quit (q)              das Programm beenden\n",
);

/// the debugger of a run
pub struct Debugger {
    /// the script's name as it was given, its source
    name: String,
    src: String,
    /// its normalised path in the VM
    main: String,
    /// standard input not yet used
    buf: Vec<u8>,
    eof: bool,
    /// breakpoints before run: (file, line, condition)
    pending: Vec<(String, u32, Option<String>)>,
}

/// the directory part of a path ("" without one)
fn dir_of(p: &str) -> &str {
    let p = p.trim_end_matches(|c| c != '/' && c != '\\');
    p.trim_end_matches(['/', '\\'])
}

impl Debugger {
    pub fn new(name: &str, src: &str) -> Debugger {
        Debugger { name: String::from(name), src: String::from(src), main: String::new(), buf: Vec::new(), eof: false, pending: Vec::new() }
    }

    /// the next line of standard input (None at its end)
    fn line(&mut self) -> Option<String> {
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let l: Vec<u8> = self.buf.drain(..=i).collect();
                return Some(String::from(String::from_utf8_lossy(&l).trim()));
            }
            if self.eof {
                if self.buf.is_empty() {
                    return None;
                }
                let l = core::mem::take(&mut self.buf);
                return Some(String::from(String::from_utf8_lossy(&l).trim()));
            }
            let mut b = [0u8; 4096];
            match plat::read_stdin(&mut b) {
                Ok(0) | Err(_) => self.eof = true,
                Ok(n) => self.buf.extend_from_slice(&b[..n]),
            }
        }
    }

    /// the next command: (word, the rest)
    fn command(&mut self) -> Option<(String, String)> {
        loop {
            out("(jfs) ");
            let l = self.line()?;
            if l.is_empty() {
                continue;
            }
            let (w, rest) = l.split_once(char::is_whitespace).unwrap_or((l.as_str(), ""));
            return Some((String::from(w), String::from(rest.trim())));
        }
    }

    /// "FILE:LINE [if COND]" or "LINE [if COND]" -> (file, line, cond); file "" is the script
    fn parse_break(&self, arg: &str) -> Result<(String, u32, Option<String>), String> {
        let (at, cond) = match arg.split_once(" if ") {
            Some((a, c)) => (a.trim(), Some(String::from(c.trim()))),
            None => (arg.trim(), None),
        };
        let (file, line) = match at.rsplit_once(':') {
            Some((f, l)) if !f.is_empty() => (f, l),
            _ => ("", at),
        };
        let Ok(line) = line.trim().parse::<u32>() else {
            return Err(format!("break braucht DATEI:ZEILE oder ZEILE, nicht \u{201e}{}\u{201c}", arg));
        };
        if let Some(c) = &cond {
            script::debug::check_expr(c).map_err(|m| format!("Bedingung: {}", m))?;
        }
        let file = if file.is_empty() || file == self.name {
            String::new()
        } else if plat::file_exists(file) != 0 || dir_of(&self.name).is_empty() {
            String::from(file)
        } else {
            format!("{}/{}", dir_of(&self.name), file)
        };
        Ok((file, line, cond))
    }

    /// the file of a breakpoint for the VM: the script's path for ""
    fn vm_file<'a>(&'a self, f: &'a str) -> &'a str {
        if f.is_empty() { &self.name } else { f }
    }

    /// the commands before run; false: quit
    pub fn prelude(&mut self) -> bool {
        out(&format!("jfs-Debugger: {} (help zeigt die Befehle, run startet das Programm)\n", self.name));
        loop {
            let Some((w, rest)) = self.command() else { return self.quit() };
            match w.as_str() {
                "run" | "r" => return true,
                "quit" | "q" => return self.quit(),
                "break" | "b" => match self.parse_break(&rest) {
                    Ok(b) => {
                        self.pending.push(b);
                        let (f, l, _) = &self.pending[self.pending.len() - 1];
                        out(&format!("Haltepunkt {}: {}:{}\n", self.pending.len(), self.vm_file(f), l));
                    }
                    Err(m) => out(&format!("{}\n", m)),
                },
                "breaks" => {
                    for (i, (f, l, c)) in self.pending.iter().enumerate() {
                        out(&format!("{}: {}:{}{}\n", i + 1, self.vm_file(f), l, c.as_ref().map_or(String::new(), |c| format!(" if {}", c))));
                    }
                }
                "help" | "h" => out(HELP),
                w => out(&format!("{} geht erst, wenn das Programm läuft (run startet es)\n", w)),
            }
        }
    }

    /// the breakpoints into a new VM (they get the ids 1, 2 .. as before run)
    pub fn attach(&mut self, vm: &mut Vm) {
        self.main = vm.modules().first().cloned().unwrap_or_default();
        for (f, l, c) in &self.pending {
            if let Err(m) = vm.set_breakpoint(self.vm_file(f), *l, c.as_deref()) {
                err(&format!("jfs: {}\n", m));
            }
        }
    }

    /// the name of a file for messages: the script's as it was given
    fn shown<'a>(&'a self, f: &'a str) -> &'a str {
        if f.is_empty() || f == self.main { &self.name } else { f }
    }

    /// the text of line n of a file
    fn source_line(&self, host: &Console, file: &str, n: u32) -> String {
        let text = if file.is_empty() || file == self.main { self.src.as_str() } else { host.sources.get(file).map_or("", |s| s.as_str()) };
        let l = text.split('\n').nth((n as usize).wrapping_sub(1)).unwrap_or("");
        String::from(l.trim_end_matches('\r'))
    }

    /// a stop of the program: the commands until it goes on
    pub fn stopped(&mut self, vm: &mut Vm, host: &Console, p: &Pause) -> Go {
        let why = match (p.reason, p.breakpoint) {
            (PauseReason::Breakpoint, Some(id)) => format!("Haltepunkt {}", id),
            (PauseReason::Breakpoint, None) => String::from("Haltepunkt"),
            (PauseReason::Step, _) => String::from("Schritt"),
            (PauseReason::Pause, _) => String::from("Pause"),
        };
        let mut s = format!("Halt in {}:{} ({})\n", self.shown(&p.file), p.line, why);
        if let Some(e) = &p.error {
            s.push_str(&format!("Die Bedingung ging nicht: {}\n", e));
        }
        s.push_str(&format!("{:>5} | {}\n", p.line, self.source_line(host, &p.file, p.line)));
        out(&s);
        loop {
            let Some((w, rest)) = self.command() else {
                self.quit();
                return Go::Quit;
            };
            match w.as_str() {
                "continue" | "c" => return Go::On,
                "step" | "s" => {
                    vm.step(Step::Into);
                    return Go::On;
                }
                "next" | "n" => {
                    vm.step(Step::Over);
                    return Go::On;
                }
                "finish" | "f" => {
                    vm.step(Step::Out);
                    return Go::On;
                }
                "quit" | "q" => {
                    self.quit();
                    return Go::Quit;
                }
                "print" | "p" => match vm.eval(0, &rest) {
                    Ok(v) => out(&format!("= {}\n", vm.repr(v))),
                    Err(m) => out(&format!("Fehler: {}\n", m)),
                },
                "bt" => {
                    let frames = vm.frames();
                    let mut s = String::new();
                    for (i, f) in frames.iter().enumerate() {
                        let func = match f.func.as_str() {
                            "" if i + 1 == frames.len() => "Hauptprogramm",
                            "" => "Modul",
                            n => n,
                        };
                        s.push_str(&format!("#{} {} ({}:{})\n", i, func, self.shown(&f.file), f.line));
                    }
                    out(&s);
                }
                "locals" => {
                    let mut s = String::new();
                    for (n, v) in vm.locals(0) {
                        s.push_str(&format!("{} = {}\n", n, vm.preview(v, 200)));
                    }
                    for (n, v) in vm.upvalues(0) {
                        s.push_str(&format!("{} = {} (erfasst)\n", n, vm.preview(v, 200)));
                    }
                    if s.is_empty() {
                        s.push_str("(keine lokalen Variablen)\n");
                    }
                    out(&s);
                }
                "break" | "b" => match self.parse_break(&rest) {
                    Ok((f, l, c)) => match vm.set_breakpoint(self.vm_file(&f), l, c.as_deref()) {
                        Ok(id) => out(&format!("Haltepunkt {}: {}:{}\n", id, self.vm_file(&f), l)),
                        Err(m) => out(&format!("{}\n", m)),
                    },
                    Err(m) => out(&format!("{}\n", m)),
                },
                "delete" | "d" => match rest.parse::<u32>() {
                    Ok(id) if vm.remove_breakpoint(id) => out(&format!("Haltepunkt {} entfernt\n", id)),
                    _ => out(&format!("Den Haltepunkt \u{201e}{}\u{201c} gibt es nicht\n", rest)),
                },
                "breaks" => {
                    let mut s = String::new();
                    for b in vm.breakpoints() {
                        let cond = b.cond.as_ref().map_or(String::new(), |c| format!(" if {}", c));
                        s.push_str(&format!("{}: {}:{}{} ({}-mal)\n", b.id, self.shown(&b.file), b.line, cond, b.hits));
                    }
                    out(&s);
                }
                "run" | "r" => out("Das Programm läuft schon\n"),
                "help" | "h" => out(HELP),
                w => out(&format!("Unbekannter Befehl {} (help zeigt die Befehle)\n", w)),
            }
        }
    }

    /// quit (or the end of the input): false for prelude
    fn quit(&self) -> bool {
        out("Das Programm endet auf Wunsch.\n");
        false
    }

    /// the program is over: with the report of its error, or at its end
    pub fn ended(&mut self, error: Option<&str>) {
        match error {
            Some(t) => {
                err(t);
                out("Das Programm endet mit einem Fehler.\n");
            }
            None => out("Das Programm ist zu Ende.\n"),
        }
    }
}
