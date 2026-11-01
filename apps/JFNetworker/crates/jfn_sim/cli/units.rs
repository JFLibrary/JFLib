// units -- systemctl and journalctl in the shell of a Linux host (A2; units.rs runs the units):
// the command words and options follow the originals, the texts and tables are our own words
//
//   systemctl start|stop|restart UNIT ...      a unit runs or not; 5 when it does not exist, 1
//                                              when a start fails
//   systemctl status [UNIT ...]                our own layout: state, file, program, restarts,
//                                              the last lines of its log; 0 running, 3 not, 4 no
//                                              such unit
//   systemctl enable|disable [--now] UNIT ...  the files that start it with the device; 1 for a
//                                              unit without a file
//   systemctl is-active|is-failed|is-enabled UNIT ...   a word per unit, -q none; 0 when the
//                                              first holds, else 3 (is-active), 1 (the others)
//   systemctl list-units [--all] [--failed]    a table of the units; list-unit-files the files
//   systemctl daemon-reload | reset-failed [UNIT] | cat UNIT
//   journalctl [-u UNIT] [-n N] [-f] [-r] [--no-pager]
//
// journalctl shows the device's log (Os::log: what every service writes, the last 1000 lines), -u
// the lines of one unit: what its service wrote, with its name as the tag or its id
// (LogLine::svc), and what the unit manager said about it.  A line reads "Jan 01 00:00:05 HOST
// TAG: TEXT", the file format of a syslog.  -f follows: the console job waits (Ctrl+C ends it, 130)
// and every new line comes as it is written, in any session and in the background.
// Scripts as units write their print output into the log as well.
//
// Tab: "systemctl " offers the verbs, after them the units of the device.  Errors go to stderr.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::unit;

use super::lab::table;
use super::print::JobOut;
use super::CliJob;
use crate::kernel::Engine;
use crate::os::{bsd_stamp, LogLine};
use crate::trace::fmt_ns;
use crate::units::{unit_journal, UnitEnabled, UnitErr, UnitOp, UnitState, UnitView};

const VERBS: &[&str] = &["start", "stop", "restart", "status", "enable", "disable", "is-active", "is-failed", "is-enabled", "list-units", "list-unit-files", "daemon-reload", "reset-failed", "cat"];

/// the lines of a unit's log a status shows
const STATUS_LINES: usize = 10;
/// journalctl's lines at most without -n when following
const FOLLOW_FIRST: usize = 10;

fn journal_line(l: &LogLine, host: &str) -> String {
    format!("{} {} {}: {}\n", bsd_stamp(l.clock), host, l.tag, l.text)
}

impl Engine {
    /// the lines of the log of unit `name` (None: all) after sequence number `after`
    fn journal_lines(&self, ni: usize, name: Option<&str>, after: u32) -> Vec<LogLine> {
        let Some(os) = self.os(ni) else { return Vec::new() };
        let svc = name.and_then(|n| self.unit_svc(ni, n));
        os.log.lines.iter().filter(|l| l.seq > after && name.is_none_or(|n| unit_journal(l, n, svc))).cloned().collect()
    }

    /// the words of systemctl's units: "web" and "web.service" are the same, a pattern matches
    /// the device's units
    fn units_of(&self, ni: usize, words: &[&str]) -> Vec<String> {
        let dev = self.dev_of(ni);
        let mut out = Vec::new();
        for w in words {
            if w.contains(['*', '?']) {
                let all = self.units(dev);
                out.extend(all.into_iter().map(|u| u.name).filter(|n| base::pattern::glob(w, n)));
            } else {
                out.push(unit::unit_name(w));
            }
        }
        out
    }

    fn sc_err(&mut self, ni: usize, code: i32, text: &str) -> Result<(), String> {
        self.sh_set_code(ni, code);
        self.sh_err(ni, text)
    }

    /// systemctl VERB ...
    pub(crate) fn sh_systemctl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut quiet, mut now, mut all, mut failed) = (false, false, false, false);
        let mut lines = STATUS_LINES;
        let mut words: Vec<&str> = Vec::new();
        let mut i = 0;
        while let Some(a) = args.get(i) {
            match *a {
                "-h" | "--help" => {
                    self.out(ni, &super::man::usage("systemctl"));
                    return Ok(());
                }
                "-q" | "--quiet" => quiet = true,
                "--now" => now = true,
                "-a" | "--all" => all = true,
                "--failed" => failed = true,
                "--no-pager" | "--no-legend" | "-l" | "--full" => {}
                "-n" | "--lines" => {
                    i += 1;
                    lines = args.get(i).and_then(|n| n.parse().ok()).unwrap_or(STATUS_LINES);
                }
                o if o.starts_with('-') => return self.sc_err(ni, 1, &format!("systemctl: die Option {} kenne ich nicht\n", o)),
                w => words.push(w),
            }
            i += 1;
        }
        let verb = words.first().copied().unwrap_or("list-units");
        let rest = if words.is_empty() { &[][..] } else { &words[1..] };
        if !VERBS.contains(&verb) {
            return self.sc_err(ni, 1, &format!("systemctl: den Befehl {} kenne ich nicht (systemctl --help)\n", verb));
        }
        let names = self.units_of(ni, rest);
        let dev = self.dev_of(ni);
        match verb {
            "list-units" => {
                self.sc_list(ni, all, failed);
                Ok(())
            }
            "list-unit-files" => {
                self.sc_files(ni);
                Ok(())
            }
            "daemon-reload" => {
                self.units_reload(ni);
                self.os_log(ni, 6, crate::units::LOG_TAG, "Einheiten neu gelesen", None);
                Ok(())
            }
            "reset-failed" => {
                let now_t = self.now;
                if let Some(os) = self.os_mut(ni) {
                    for (n, u) in os.units.map.iter_mut() {
                        if u.state == UnitState::Failed && (names.is_empty() || names.contains(n)) {
                            u.state = UnitState::Inactive;
                            u.since = now_t;
                            u.why = None;
                        }
                    }
                }
                Ok(())
            }
            "status" => self.sc_status(ni, &names, lines),
            _ if names.is_empty() => self.sc_err(ni, 1, &format!("systemctl {}: welche Einheit? (systemctl {} NAME)\n", verb, verb)),
            "start" | "stop" | "restart" => {
                let op = match verb {
                    "start" => UnitOp::Start,
                    "stop" => UnitOp::Stop,
                    _ => UnitOp::Restart,
                };
                let mut first = Ok(());
                for n in &names {
                    if let Err(e) = self.unit_op(dev, n, op) {
                        let code = if matches!(e, UnitErr::NotFound(_)) { 5 } else { 1 };
                        let hint = if matches!(e, UnitErr::Failed(_)) { format!(" (systemctl status {}, journalctl -u {})", n, n) } else { String::new() };
                        let r = self.sc_err(ni, code, &format!("systemctl {}: {}{}\n", verb, e.msg().text(), hint));
                        if first.is_ok() {
                            first = r;
                        }
                    }
                }
                first
            }
            "enable" | "disable" => {
                let on = verb == "enable";
                let mut first = Ok(());
                for n in &names {
                    let r = if on { self.unit_enable(ni, n) } else { self.unit_disable(ni, n) };
                    match r {
                        Ok(files) => {
                            if files.is_empty() && on {
                                let has = self.unit_enabled(ni, n) == Some(UnitEnabled::Enabled);
                                if !has {
                                    self.out(ni, &format!("{} hat kein [Install] mit WantedBy=: nichts einzuschalten.\n", n));
                                }
                            }
                            for f in files {
                                self.out(ni, &if on { format!("eingeschaltet: {}\n", f) } else { format!("ausgeschaltet: {}\n", f) });
                            }
                            if now {
                                let op = if on { UnitOp::Start } else { UnitOp::Stop };
                                if let Err(e) = self.unit_op(dev, n, op)
                                    && first.is_ok()
                                {
                                    first = self.sc_err(ni, 1, &format!("systemctl {}: {}\n", verb, e.msg().text()));
                                }
                            }
                        }
                        Err(e) => {
                            let r = self.sc_err(ni, 1, &format!("systemctl {}: {}\n", verb, e.msg().text()));
                            if first.is_ok() {
                                first = r;
                            }
                        }
                    }
                }
                first
            }
            "is-active" | "is-failed" | "is-enabled" => {
                let mut ok = false;
                for n in &names {
                    let v = self.unit(dev, n);
                    let (word, holds) = match verb {
                        "is-active" => {
                            let st = v.as_ref().map_or(UnitState::Inactive, |u| u.state);
                            (st.active_word(), st == UnitState::Active)
                        }
                        "is-failed" => {
                            let st = v.as_ref().map_or(UnitState::Inactive, |u| u.state);
                            (st.active_word(), st == UnitState::Failed)
                        }
                        _ => match v.as_ref().map(|u| u.enabled) {
                            Some(e) => (e.word(), e.holds()),
                            None => ("not-found", false),
                        },
                    };
                    ok |= holds;
                    if !quiet {
                        self.out(ni, &format!("{}\n", word));
                    }
                }
                if ok {
                    return Ok(());
                }
                self.sh_set_code(ni, if verb == "is-active" { 3 } else { 1 });
                Err(String::new())
            }
            "cat" => {
                let mut first = Ok(());
                for n in &names {
                    let text = self.os(ni).and_then(|o| o.files.read(&unit::path_of(n)).ok().map(|b| String::from_utf8_lossy(b).into_owned()));
                    match (text, self.unit(dev, n)) {
                        (Some(t), _) => {
                            let tail = if t.ends_with('\n') { "" } else { "\n" };
                            self.out(ni, &format!("# {}\n{}{}", unit::path_of(n), t, tail));
                        }
                        (None, Some(u)) => self.out(ni, &format!("# erzeugt: ein Dienst des Projekts\n[Service]\nExecStart=jfn-service {}\n", u.exec)),
                        (None, None) => {
                            let r = self.sc_err(ni, 1, &format!("systemctl cat: {}\n", UnitErr::NotFound(n.clone()).msg().text()));
                            if first.is_ok() {
                                first = r;
                            }
                        }
                    }
                }
                first
            }
            _ => Ok(()),
        }
    }

    /// list-units: our own table
    fn sc_list(&mut self, ni: usize, all: bool, failed: bool) {
        let dev = self.dev_of(ni);
        let units: Vec<UnitView> = self.units(dev).into_iter().filter(|u| if failed { u.state == UnitState::Failed } else { all || u.state != UnitState::Inactive }).collect();
        if units.is_empty() {
            self.out(ni, "Keine Einheiten in diesem Zustand (--all zeigt alle).\n");
            return;
        }
        let mut rows = alloc::vec![super::lab::row(&["Einheit", "Quelle", "Aktiv", "Im Detail", "Beschreibung"])];
        for u in &units {
            let src = match (u.bad, u.file.is_some()) {
                (true, _) => "Fehler",
                (_, true) => "Datei",
                _ => "Projekt",
            };
            rows.push(alloc::vec![u.name.clone(), String::from(src), String::from(u.state.active_word()), String::from(u.state.sub_word()), u.description.clone()]);
        }
        let n = units.len();
        let mut s = table(&rows);
        s.push_str(&if n == 1 { String::from("1 Einheit\n") } else { format!("{} Einheiten\n", n) });
        self.out(ni, &s);
    }

    /// list-unit-files: our own table
    fn sc_files(&mut self, ni: usize) {
        let dev = self.dev_of(ni);
        let mut rows = alloc::vec![super::lab::row(&["Einheit", "Beim Start", "Datei"])];
        let units = self.units(dev);
        for u in &units {
            rows.push(alloc::vec![u.name.clone(), String::from(u.enabled.word()), u.file.clone().unwrap_or_else(|| String::from("(aus dem Projekt)"))]);
        }
        let mut s = table(&rows);
        s.push_str(&format!("{} Einheiten\n", units.len()));
        self.out(ni, &s);
    }

    /// status: our own layout; 0 when the (first) unit runs, 3 when not, 4 when there is none
    fn sc_status(&mut self, ni: usize, names: &[String], lines: usize) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let host = self.host(ni);
        if names.is_empty() {
            let mut s = format!("{}: {} Einheiten", host, self.units(dev).len());
            let failed = self.units(dev).iter().filter(|u| u.state == UnitState::Failed).count();
            let up = self.units(dev).iter().filter(|u| u.state == UnitState::Active).count();
            s.push_str(&format!(", {} laufen, {} mit Fehler\n", up, failed));
            self.out(ni, &s);
            return Ok(());
        }
        let mut code = 0;
        for (k, n) in names.iter().enumerate() {
            if k > 0 {
                self.out(ni, "\n");
            }
            let Some(u) = self.unit(dev, n) else {
                self.err_out(ni, &format!("{}\n", UnitErr::NotFound(n.clone()).msg().text()));
                code = code.max(4);
                continue;
            };
            let word = match u.state {
                UnitState::Active => "läuft",
                UnitState::Inactive => "gestoppt",
                UnitState::Failed => "Fehler",
                UnitState::AutoRestart => "wartet auf den Neustart",
            };
            let mut s = String::new();
            let d = if u.description.is_empty() { String::new() } else { format!(": {}", u.description) };
            s.push_str(&format!("{}{}\n", u.name, d));
            // (nothing ever happened to a unit that never ran)
            let since = if u.state == UnitState::Inactive && u.since == 0 { String::new() } else { format!(" seit {}", fmt_ns(u.since as i64)) };
            s.push_str(&format!("  Zustand:   {} ({}/{}){}\n", word, u.state.active_word(), u.state.sub_word(), since));
            s.push_str(&format!("  Quelle:    {} (beim Start: {})\n", u.file.as_deref().unwrap_or("aus dem Projekt"), u.enabled.word()));
            if u.file.is_some() {
                s.push_str(&format!("  Programm:  {}\n", u.exec));
                match u.restart {
                    unit::Restart::No => s.push_str("  Neustart:  no\n"),
                    r => s.push_str(&format!("  Neustart:  {} nach {}, bisher {}\n", r.as_str(), crate::units::span_text(u.restart_ns), u.restarts)),
                }
            } else {
                s.push_str(&format!("  Art:       {}\n", u.exec));
            }
            if !u.requires.is_empty() {
                s.push_str(&format!("  Braucht:   {}\n", u.requires.join(", ")));
            }
            if let Some(p) = u.pid {
                s.push_str(&format!("  PID:       {}\n", p));
            }
            if let Some(x) = u.exit {
                s.push_str(&format!("  Status:    {}\n", x));
            }
            if let Some(w) = &u.why {
                s.push_str(&format!("  Grund:     {}\n", w));
            }
            if u.changed {
                s.push_str("  Achtung:   die Datei ist neuer, systemctl daemon-reload liest sie\n");
            }
            for i in &u.issues {
                s.push_str(&format!("  {}: {}\n", if i.error { "Fehler" } else { "Warnung" }, i.msg.text()));
            }
            let log = self.journal_lines(ni, Some(n), 0);
            if !log.is_empty() && lines > 0 {
                s.push('\n');
                for l in log.iter().skip(log.len().saturating_sub(lines)) {
                    s.push_str(&journal_line(l, &host));
                }
            }
            self.out(ni, &s);
            if u.state != UnitState::Active {
                code = code.max(3);
            }
        }
        if code != 0 {
            self.sh_set_code(ni, code);
            return Err(String::new());
        }
        Ok(())
    }

    /// journalctl [-u UNIT] [-n N] [-f] [-r]
    pub(crate) fn sh_journalctl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut name, mut last, mut follow, mut reverse): (Option<String>, Option<usize>, bool, bool) = (None, None, false, false);
        let mut i = 0;
        while let Some(a) = args.get(i) {
            match *a {
                "-h" | "--help" => {
                    self.out(ni, &super::man::usage("journalctl"));
                    return Ok(());
                }
                "-u" | "--unit" => {
                    i += 1;
                    let Some(n) = args.get(i) else {
                        self.sh_set_code(ni, 1);
                        return self.sh_err(ni, "journalctl: -u braucht den Namen einer Einheit\n");
                    };
                    name = Some(unit::unit_name(n));
                }
                "-n" | "--lines" => {
                    i += 1;
                    match args.get(i).and_then(|n| n.parse::<usize>().ok()) {
                        Some(n) => last = Some(n),
                        None => {
                            self.sh_set_code(ni, 1);
                            return self.sh_err(ni, "journalctl: -n braucht eine Zahl\n");
                        }
                    }
                }
                "-f" | "--follow" => follow = true,
                "-r" | "--reverse" => reverse = true,
                "--no-pager" | "-e" | "-b" => {}
                o if o.starts_with("-n") && o[2..].parse::<usize>().is_ok() => last = o[2..].parse().ok(),
                o if o.starts_with("-u") && o.len() > 2 => name = Some(unit::unit_name(&o[2..])),
                o => {
                    self.sh_set_code(ni, 1);
                    return self.sh_err(ni, &format!("journalctl: {} kenne ich nicht\n", o));
                }
            }
            i += 1;
        }
        let host = self.host(ni);
        let lines = self.journal_lines(ni, name.as_deref(), 0);
        let n = match (last, follow) {
            (Some(k), _) => k,
            (None, true) => FOLLOW_FIRST,
            (None, false) => lines.len(),
        };
        let mark = self.os(ni).map_or(0, |o| o.log.lines.back().map_or(0, |l| l.seq));
        let from = lines.len().saturating_sub(n);
        let mut s = String::new();
        if reverse {
            for l in lines[from..].iter().rev() {
                s.push_str(&journal_line(l, &host));
            }
        } else {
            for l in &lines[from..] {
                s.push_str(&journal_line(l, &host));
            }
        }
        if s.is_empty() && !follow {
            s.push_str("Keine Zeilen.\n");
        }
        self.out(ni, &s);
        if follow {
            self.next_job += 1;
            let id = self.next_job;
            let start = self.now;
            if let Some(c) = self.cdev_mut(ni) {
                c.job = Some(CliJob { id, out: JobOut::Journal { unit: name, mark }, start });
            }
        }
        Ok(())
    }

    /// a line went into the log: every journalctl -f of the device, in any session and in the
    /// background, prints what it has not shown yet
    pub(crate) fn journal_follow(&mut self, ni: usize) {
        let Some(c) = self.cdev(ni) else { return };
        let is = |j: &Option<CliJob>| matches!(j, Some(CliJob { out: JobOut::Journal { .. }, .. }));
        let here_fg = is(&c.job);
        let here_bg: Vec<u32> = c.sh.bg.iter().filter(|b| is(&b.cj)).map(|b| b.no).collect();
        let others: Vec<(u32, bool, Vec<u32>)> = c.parked.iter().map(|(s, p)| (*s, is(&p.job), p.sh.bg.iter().filter(|b| is(&b.cj)).map(|b| b.no).collect())).filter(|x: &(u32, bool, Vec<u32>)| x.1 || !x.2.is_empty()).collect();
        if !here_fg && here_bg.is_empty() && others.is_empty() {
            return;
        }
        // (a background job's console job stands in CliDev::job while it is worked on)
        if here_fg {
            self.journal_print(ni);
        }
        for no in here_bg {
            self.sh_bg_event(ni, no, |e| e.journal_print(ni));
        }
        for (s, fg, bg) in others {
            self.in_ses(ni, s, |e| {
                if fg {
                    e.journal_print(ni);
                }
                for no in bg {
                    e.sh_bg_event(ni, no, |e| e.journal_print(ni));
                }
            });
        }
    }

    /// the new lines for the console job in place
    fn journal_print(&mut self, ni: usize) {
        let Some(CliJob { out: JobOut::Journal { unit, mark }, .. }) = self.cdev(ni).and_then(|c| c.job.clone()) else {
            return;
        };
        let lines = self.journal_lines(ni, unit.as_deref(), mark);
        let top = self.os(ni).map_or(mark, |o| o.log.lines.back().map_or(mark, |l| l.seq));
        if let Some(CliJob { out: JobOut::Journal { mark, .. }, .. }) = self.cdev_mut(ni).and_then(|c| c.job.as_mut()) {
            *mark = top;
        }
        if lines.is_empty() {
            return;
        }
        let host = self.host(ni);
        let mut s = String::new();
        for l in &lines {
            s.push_str(&journal_line(l, &host));
        }
        self.out(ni, &s);
    }

    /// Tab after "systemctl": the verbs, then the device's units
    pub(crate) fn systemctl_complete(&self, ni: usize, line: &str) -> Option<(Option<String>, Vec<String>)> {
        let w: Vec<&str> = line.split_whitespace().collect();
        if w.first().copied() != Some("systemctl") {
            return None;
        }
        let open = line.ends_with(char::is_whitespace);
        let (last, n) = if open { ("", w.len()) } else { (w.last().copied().unwrap_or(""), w.len() - 1) };
        if n == 0 {
            return None;
        }
        let dev = self.dev_of(ni);
        let mut all: Vec<String> = if n == 1 { VERBS.iter().map(|v| String::from(*v)).collect() } else { self.units(dev).into_iter().map(|u| u.name).collect() };
        all.retain(|x| x.starts_with(last));
        all.sort();
        let head = &line[..line.len() - last.len()];
        let done = if all.len() == 1 { Some(format!("{}{} ", head, all[0])) } else { None };
        Some((done, all))
    }
}
