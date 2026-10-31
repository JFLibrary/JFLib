// shrun -- the shell runs its lines (cli/sh.rs parses them): lists, and-or lists, pipelines and
// redirections over virtual time, background jobs and processes
//
//   Engine::sh_line(ni, line)        a line typed at the shell (or a line of the startup
//                                    configuration): !!, the history, the grammar, the run;
//                                    Err with the first error line when its last command failed
//   Run                              a line on its way (Shell::fg; a background job has its
//                                    own): the command that runs, where its output goes (Sink),
//                                    its input, the files of its redirections
//   BgJob                            "CMD &": number, PID, text, its Run and its console job
//   Engine::sh_sink(ni, text, err)   out() and err_out() ask it first: a pipe or a redirection of
//                                    the command that runs takes the text
//   Engine::err_out(ni, text)        what a command writes to stderr (2>, 2>&1); sh_stdout_term:
//                                    the command writes to the terminal (ls lays out columns)
//   Engine::cli_pager(dev)           the last line asked the terminal to page (less, more, man;
//                                    doc/console.rs pages with --More--); cli_cwd(dev) the
//                                    session's working directory
//   Engine::sh_job_ended / sh_go     cli_job_done: the command's console job ended, the line goes
//                                    on; sh_interrupted (Ctrl+C), sh_abort (boot), sh_power_off
//   jobs, fg, wait, kill, ps, sleep, jfs   the commands of processes
//   jfs --test FILE ...              the test functions of files (cli/jfstest.rs, IDE1)
//   JfsProc                          `jfs FILE [ARGS]`: a script of the device's files run as a
//                                    console program (the ScriptService of scriptsvc.rs and its
//                                    host: print to the terminal, log to the journal, net, http,
//                                    timer, disk as for services, config.args from ARGS); it ends
//                                    when nothing is left to do (0) or after an error (1)
//
// Pipes collect: the left side's output becomes the right side's input when the left side ends
// (a ping on the left of a pipe prints when it is done); a redirection into a file writes when
// its command ends (a file that cannot be written is refused before the command runs).  A
// command that runs over time (ping, curl, sleep, jfs ...) holds the line; its exit code comes
// when it ends.  Ctrl+C ends the command that runs: the rest of its pipeline still filters what
// was collected, the rest of the line is dropped, $? is 130.
//
// Background jobs run their and-or list as the foreground does, while the console takes lines;
// their output reaches the terminal (and their pipes and redirections) as it comes.  "[N] PID"
// comes first, the line "[N]+  Done  ..." before the next prompt after the job ended (as bash
// prints them).  A job's console job sits in its BgJob; while the engine works on it (an event
// of its ping, its program's output or exit, a timer) it is swapped into CliDev::job and
// EngineCli::sh_ctx names the job, so the console jobs of print.rs, tools.rs and the others
// work unchanged; their wake-ups carry BG_WAKE | job number in Ev::Cli (Ev::Ses in another
// session than the console: cli/ses.rs).
//
// PIDs: a process that runs as a service (a console program, jfs) is 100 + its service id, as
// top and ss -p show it; a background job that is none gets a service id of its own
// (Os::take_id).  ps lists init (1), the shell (100; the shells of further sessions have PIDs of
// their own: cli/ses.rs), the running services and the background jobs of every session, with
// their terminal (TTY: tty1 for the console, pts/N, "?" for init and services); kill ends a
// background job (TERM 143, KILL 137, INT 130, HUP 129), services are not
// processes the shell may end.  A device switched off loses its shell: jobs, line, history.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use super::print::JobOut;
use super::sh::{self, AndOr, Conn, ROp, Redir, Shell, Var, SHELL_PID};
use super::CliJob;
use crate::jobs::JobKind;
use crate::kernel::{Engine, Ns};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::scriptsvc::{ScriptCfg, ScriptService, ScriptState};

/// Ev::Cli epochs with this bit wake a background job (the rest is its number)
pub const BG_WAKE: u32 = 0x8000_0000;

/// where a command's output goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink {
    /// the terminal
    Term,
    /// the next command of the pipeline
    Pipe,
    /// a file of the redirections (Run::files)
    File(usize),
    /// /dev/null
    Null,
}

/// a file a redirection writes when its command ends
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutFile {
    /// as typed (messages), absolute
    pub path: String,
    pub abs: String,
    pub append: bool,
    pub data: Vec<u8>,
}

/// where a run is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// the next command starts
    Next,
    /// a command runs over time (its console job is CliDev::job)
    Wait,
    /// the command ended with this status
    Ended(i32),
    /// nothing is left
    Done,
}

/// a line on its way
#[derive(Clone, Debug)]
pub struct Run {
    pub items: Vec<AndOr>,
    pub item: usize,
    pub part: usize,
    pub stage: usize,
    pub phase: Phase,
    /// what the running command reads (the previous command's output, a file of <)
    pub input: Option<Vec<u8>>,
    /// what it writes into the pipe
    pub pipe: Vec<u8>,
    pub out: Sink,
    pub err: Sink,
    pub files: Vec<OutFile>,
    /// the status of the last pipeline
    pub code: i32,
    /// a command is being started (calls back in wait for it)
    pub busy: bool,
    /// Ctrl+C: the rest of the pipeline filters, the rest of the line goes
    pub interrupted: bool,
    /// kill: the signal that ended the job
    pub killed: Option<u8>,
    /// a background job's output before its "[N] PID" line
    pub hold: Option<String>,
    /// NAME=value before a command: what to set back after it
    pub undo: Vec<(String, Option<Var>)>,
    /// the first line the running command wrote to stderr, the one of the last failure
    pub cur_err: Option<String>,
    pub err_line: Option<String>,
}

impl Run {
    pub fn new(items: Vec<AndOr>) -> Run {
        Run {
            items,
            item: 0,
            part: 0,
            stage: 0,
            phase: Phase::Next,
            input: None,
            pipe: Vec::new(),
            out: Sink::Term,
            err: Sink::Term,
            files: Vec::new(),
            code: 0,
            busy: false,
            interrupted: false,
            killed: None,
            hold: None,
            undo: Vec::new(),
            cur_err: None,
            err_line: None,
        }
    }
    pub fn footprint(&self) -> usize {
        let mut b = core::mem::size_of::<Run>() + self.pipe.len() + self.input.as_ref().map_or(0, |i| i.len());
        b += self.files.iter().map(|f| f.data.len() + f.path.len() + f.abs.len() + 64).sum::<usize>();
        b += self.items.iter().map(|i| i.text.len() * 2 + 128).sum::<usize>();
        b
    }
}

/// a job in the background
#[derive(Clone, Debug)]
pub struct BgJob {
    pub no: u32,
    pub pid: u32,
    pub text: String,
    pub run: Run,
    /// its console job while it waits for one (swapped into CliDev::job while it is worked on)
    pub cj: Option<CliJob>,
    pub start: Ns,
    /// it ended: "Done", "Exit 1", "Terminated" ...
    pub ended: Option<String>,
    /// `fg` / `wait` waits for it in the foreground (its end prints no note)
    pub waited: bool,
}

/// the words of a signal: its number, its name
const SIGNALS: [(u8, &str); 5] = [(1, "HUP"), (2, "INT"), (9, "KILL"), (15, "TERM"), (0, "0")];

fn signal_of(s: &str) -> Option<u8> {
    let s = s.strip_prefix("SIG").unwrap_or(s);
    if let Ok(n) = s.parse::<u8>() {
        return SIGNALS.iter().find(|x| x.0 == n).map(|x| x.0);
    }
    SIGNALS.iter().find(|x| x.1.eq_ignore_ascii_case(s)).map(|x| x.0)
}

/// what bash shows for a job ended by a signal
fn signal_word(sig: u8) -> &'static str {
    match sig {
        1 => "Hangup",
        2 => "Interrupt",
        9 => "Killed",
        _ => "Terminated",
    }
}

/// "m:ss" of a CPU time (ps)
fn cpu_time(ns: u64) -> String {
    let s = ns / 1_000_000_000;
    format!("{}:{:02}", s / 60, s % 60)
}

impl Engine {
    // ------------------------------------------------------------ the context

    /// the background job the engine works on at node `ni`, if any
    pub(crate) fn sh_bg_ctx(&self, ni: usize) -> Option<u32> {
        self.cli.sh_ctx.filter(|c| c.0 == ni).map(|c| c.1)
    }

    /// the run of the context: the background job's, else the foreground line's
    pub(crate) fn sh_run(&self, ni: usize) -> Option<&Run> {
        let bg = self.sh_bg_ctx(ni);
        let s = self.sh(ni)?;
        match bg {
            Some(no) => s.bg.iter().find(|j| j.no == no).map(|j| &j.run),
            None => s.fg.as_ref(),
        }
    }
    pub(crate) fn sh_run_mut(&mut self, ni: usize) -> Option<&mut Run> {
        let bg = self.sh_bg_ctx(ni);
        let s = self.sh_mut(ni)?;
        match bg {
            Some(no) => s.bg.iter_mut().find(|j| j.no == no).map(|j| &mut j.run),
            None => s.fg.as_mut(),
        }
    }

    // ------------------------------------------------------------ output

    /// a pipe, a redirection or a held first output takes the text (true), else it goes to
    /// the terminal
    pub(crate) fn sh_sink(&mut self, ni: usize, text: &str, err: bool) -> bool {
        let Some(run) = self.sh_run_mut(ni) else { return false };
        if err && run.cur_err.is_none() {
            run.cur_err = Some(String::from(text.lines().next().unwrap_or("")));
        }
        match if err { run.err } else { run.out } {
            Sink::Term => match run.hold.as_mut() {
                Some(h) => {
                    h.push_str(text);
                    true
                }
                None => false,
            },
            Sink::Pipe => {
                run.pipe.extend_from_slice(text.as_bytes());
                true
            }
            Sink::File(k) => {
                if let Some(f) = run.files.get_mut(k) {
                    f.data.extend_from_slice(text.as_bytes());
                }
                true
            }
            Sink::Null => true,
        }
    }

    /// what a command writes to stderr
    pub(crate) fn err_out(&mut self, ni: usize, text: &str) {
        if text.is_empty() || self.sh_sink(ni, text, true) {
            return;
        }
        self.out_raw(ni, text);
    }

    /// the shell's own lines (notes, "[1] 105", errors of redirections): the terminal
    fn term_out(&mut self, ni: usize, text: &str) {
        if let Some(h) = self.sh_run_mut(ni).and_then(|r| r.hold.as_mut()) {
            h.push_str(text);
            return;
        }
        self.out_raw(ni, text);
    }

    /// an error of the shell itself (Err for the startup configuration)
    fn sh_fail(&mut self, ni: usize, text: &str) -> String {
        if let Some(r) = self.sh_run_mut(ni)
            && r.cur_err.is_none()
        {
            r.cur_err = Some(String::from(text.lines().next().unwrap_or("")));
        }
        self.term_out(ni, text);
        String::from(text.lines().next().unwrap_or(""))
    }

    /// a command's exit status when it is not 0 / 1 (grep without a match, a usage error)
    pub(crate) fn sh_set_code(&mut self, ni: usize, code: i32) {
        if let Some(s) = self.sh_mut(ni) {
            s.code = Some(code);
        }
    }

    /// the running command writes to the terminal (ls lays out columns there, one name a line
    /// into a pipe or a file, as GNU ls does)
    pub(crate) fn sh_stdout_term(&self, ni: usize) -> bool {
        self.sh_run(ni).is_none_or(|r| r.out == Sink::Term)
    }

    /// what the running command reads (None: the console, nobody typed anything for it)
    pub(crate) fn sh_take_stdin(&mut self, ni: usize) -> Option<Vec<u8>> {
        self.sh_run_mut(ni).and_then(|r| r.input.take())
    }

    /// the last line of a device's shell asks the terminal to page what it printed (less,
    /// more, man; the console shows it with --More--)
    pub fn cli_pager(&self, dev: u32) -> bool {
        self.node_index(dev).and_then(|ni| self.sh(ni)).is_some_and(|s| s.pager)
    }

    /// the working directory of a device's console session ("/root", "flash:/")
    pub fn cli_cwd(&self, dev: u32) -> String {
        self.node_index(dev).and_then(|ni| self.sh(ni)).map(|s| s.cwd.clone()).unwrap_or_default()
    }

    /// the lines for the prompt: background jobs that ended
    fn sh_notes(&mut self, ni: usize) {
        let notes = self.sh_mut(ni).map(|s| core::mem::take(&mut s.notes)).unwrap_or_default();
        for n in notes {
            self.out_raw(ni, &format!("{}\n", n));
        }
    }

    // ------------------------------------------------------------ a line

    /// a line of the shell
    pub(crate) fn sh_line(&mut self, ni: usize, line: &str) -> Result<(), String> {
        let booting = self.cdev(ni).is_some_and(|c| c.booting);
        if let Some(s) = self.sh_mut(ni) {
            s.pager = false;
            s.code = None;
        }
        let mut line = String::from(line);
        if !booting {
            let hist: Vec<String> = self.sh(ni).map(|s| s.history.clone()).unwrap_or_default();
            match sh::expand_history(&line, &hist) {
                Ok(Some(l)) => {
                    // bash shows the line it runs
                    self.out_raw(ni, &format!("{}\n", l));
                    line = l;
                }
                Ok(None) => {}
                Err(e) => {
                    self.out_raw(ni, &e);
                    if let Some(s) = self.sh_mut(ni) {
                        s.status = 1;
                    }
                    self.sh_notes(ni);
                    return Err(String::from(e.trim_end()));
                }
            }
            if !line.trim().is_empty()
                && let Some(s) = self.sh_mut(ni)
            {
                if s.history.len() >= sh::HISTORY_MAX {
                    s.history.remove(0);
                }
                s.history.push(line.clone());
            }
        }
        let items = match sh::parse(&line) {
            Ok(v) => v,
            Err(e) => {
                self.out_raw(ni, &e);
                if let Some(s) = self.sh_mut(ni) {
                    s.status = 2;
                }
                self.sh_notes(ni);
                return Err(String::from(e.trim_end()));
            }
        };
        if items.is_empty() {
            self.sh_notes(ni);
            return Ok(());
        }
        if let Some(s) = self.sh_mut(ni) {
            s.fg = Some(Run::new(items));
            s.err_line = None;
        }
        self.sh_go(ni);
        match self.sh(ni) {
            Some(s) if s.fg.is_none() && s.status != 0 => Err(s.err_line.clone().unwrap_or_default()),
            _ => Ok(()),
        }
    }

    /// the run of the context goes on as far as it can without waiting
    pub(crate) fn sh_go(&mut self, ni: usize) {
        loop {
            let has_job = self.cdev(ni).is_some_and(|c| c.job.is_some());
            let Some(run) = self.sh_run_mut(ni) else { return };
            if run.busy {
                return;
            }
            match run.phase {
                Phase::Wait => {
                    if has_job {
                        return;
                    }
                    // the console job went without a word: it ended
                    run.phase = Phase::Ended(0);
                }
                Phase::Ended(code) => self.sh_stage_end(ni, code),
                Phase::Done => {
                    self.sh_finish(ni);
                    return;
                }
                Phase::Next => {
                    let bg = run.part == 0 && run.stage == 0 && run.items.get(run.item).is_some_and(|i| i.bg);
                    if bg {
                        self.sh_bg_start(ni);
                        continue;
                    }
                    run.busy = true;
                    run.phase = Phase::Wait;
                    let code = self.sh_stage(ni);
                    let job = self.cdev(ni).and_then(|c| c.job.as_ref()).map(|j| (j.id, j.out.clone()));
                    let interrupted = self.sh_run(ni).is_some_and(|r| r.interrupted);
                    if let (Some((id, out)), true) = (&job, interrupted) {
                        // after Ctrl+C nothing new runs over time
                        let _ = self.cdev_mut(ni).and_then(|c| c.job.take());
                        self.cancel_job(*id);
                        self.cli.jobs.remove(id);
                        if let JobOut::Proc { svc, .. } = out {
                            self.console_kill(ni, *svc);
                        }
                    }
                    let Some(run) = self.sh_run_mut(ni) else { return };
                    run.busy = false;
                    if run.phase == Phase::Wait {
                        run.phase = if job.is_some() && !interrupted { Phase::Wait } else { Phase::Ended(if job.is_some() { 130 } else { code }) };
                    }
                }
            }
        }
    }

    /// starts the command the run is at; its status when it ends at once
    fn sh_stage(&mut self, ni: usize) -> i32 {
        let Some(run) = self.sh_run_mut(ni) else { return 0 };
        let Some((_, p)) = run.items.get(run.item).and_then(|i| i.parts.get(run.part)) else { return 0 };
        let Some(cmd) = p.cmds.get(run.stage).cloned() else { return 0 };
        let last = run.stage + 1 == p.cmds.len();
        run.out = if last { Sink::Term } else { Sink::Pipe };
        run.err = Sink::Term;
        run.cur_err = None;
        if let Some(s) = self.sh_mut(ni) {
            s.code = None;
        }
        // NAME=value words first, then the command's words
        let mut assigns: Vec<(String, String)> = Vec::new();
        let mut argv: Vec<String> = Vec::new();
        for w in &cmd.words {
            if argv.is_empty()
                && let Some((n, v)) = sh::assignment(w)
            {
                let val = self.sh_expand(ni, v).join(" ");
                assigns.push((String::from(n), val));
                continue;
            }
            argv.extend(self.sh_expand(ni, w));
        }
        for r in &cmd.redirs {
            if let Err(code) = self.sh_redirect(ni, r) {
                return code;
            }
        }
        if argv.is_empty() {
            if let Some(s) = self.sh_mut(ni) {
                for (n, v) in assigns {
                    s.set_var(&n, &v);
                }
            }
            return 0;
        }
        // NAME=value cmd: the variable is exported for this command only
        let mut undo = Vec::new();
        if let Some(s) = self.sh_mut(ni) {
            for (n, v) in assigns {
                undo.push((n.clone(), s.vars.get(&n).cloned()));
                s.vars.insert(n, Var { value: v, export: true });
            }
        }
        let r = self.linux_cmd(ni, &argv);
        if let Some(s) = self.sh_mut(ni) {
            for (n, old) in undo.into_iter().rev() {
                match old {
                    Some(v) => {
                        s.vars.insert(n, v);
                    }
                    None => {
                        s.vars.remove(&n);
                    }
                }
            }
        }
        let code = self.sh_mut(ni).and_then(|s| s.code.take());
        match r {
            Ok(()) => code.unwrap_or(0),
            Err(_) => code.unwrap_or(1),
        }
    }

    /// a redirection of the command that starts; Err(status) when it cannot be done
    fn sh_redirect(&mut self, ni: usize, r: &Redir) -> Result<(), i32> {
        if r.op == ROp::Dup {
            let Some(run) = self.sh_run_mut(ni) else { return Ok(()) };
            let to = if r.word == "2" { run.err } else { run.out };
            match r.fd {
                1 => run.out = to,
                2 => run.err = to,
                _ => {}
            }
            return Ok(());
        }
        let w = self.sh_expand(ni, &r.word);
        if w.len() != 1 {
            self.sh_fail(ni, &format!("jfn-sh: {}: ambiguous redirect\n", r.word));
            return Err(1);
        }
        let path = w[0].clone();
        if r.op == ROp::In {
            let data = if path == "/dev/null" {
                Ok(Vec::new())
            } else {
                let abs = self.sh_abs(ni, &path);
                self.sh_read(ni, &abs)
            };
            return match data {
                Ok(d) => {
                    if let Some(run) = self.sh_run_mut(ni) {
                        run.input = Some(d);
                    }
                    Ok(())
                }
                Err(e) => {
                    self.sh_fail(ni, &format!("jfn-sh: {}: {}\n", path, e));
                    Err(1)
                }
            };
        }
        let append = matches!(r.op, ROp::Append | ROp::BothAppend);
        let sink = if path == "/dev/null" {
            Sink::Null
        } else {
            let abs = self.sh_abs(ni, &path);
            if let Err(e) = self.sh_writable(ni, &abs) {
                self.sh_fail(ni, &format!("jfn-sh: {}: {}\n", path, e));
                return Err(1);
            }
            let Some(run) = self.sh_run_mut(ni) else { return Ok(()) };
            run.files.push(OutFile { path, abs, append, data: Vec::new() });
            Sink::File(run.files.len() - 1)
        };
        let Some(run) = self.sh_run_mut(ni) else { return Ok(()) };
        match r.op {
            ROp::Both | ROp::BothAppend => {
                run.out = sink;
                run.err = sink;
            }
            _ if r.fd == 2 => run.err = sink,
            _ => run.out = sink,
        }
        Ok(())
    }

    /// the bytes of a file for < and the tools (the made files of /etc too)
    pub(crate) fn sh_read(&self, ni: usize, abs: &str) -> Result<Vec<u8>, &'static str> {
        let Some(st) = self.store(ni) else { return Err("No such file or directory") };
        let p = st.norm(abs).map_err(|e| e.errno_text())?;
        if let Some(t) = self.cat_etc(ni, &p) {
            return Ok(t.into_bytes());
        }
        match st.read(&p) {
            Ok(b) => Ok(Vec::from(b)),
            Err(jfn_model::files::FsErr::ReadOnly) => Err("Permission denied"),
            Err(e) => Err(e.errno_text()),
        }
    }

    /// a file can be written at this path (its directory exists, it is no directory)
    pub(crate) fn sh_writable(&self, ni: usize, abs: &str) -> Result<(), &'static str> {
        let Some(st) = self.store(ni) else { return Err("No such file or directory") };
        let p = st.norm(abs).map_err(|e| e.errno_text())?;
        if st.is_dir(&p) {
            return Err("Is a directory");
        }
        let d = st.parent(&p);
        match st.stat(&d) {
            Some(s) if s.dir => Ok(()),
            Some(_) => Err("Not a directory"),
            None => Err("No such file or directory"),
        }
    }

    /// the command ended: its files, then the next command, pipeline or and-or list
    fn sh_stage_end(&mut self, ni: usize, code: i32) {
        let files = match self.sh_run_mut(ni) {
            Some(run) => {
                run.out = Sink::Term;
                run.err = Sink::Term;
                core::mem::take(&mut run.files)
            }
            None => return,
        };
        // a file that could not take the output fails the command (the resolver's settings)
        let mut code = code;
        for f in files {
            if !self.sh_write_file(ni, f) {
                code = code.max(1);
            }
        }
        let fg = self.sh_bg_ctx(ni).is_none();
        let Some(run) = self.sh_run_mut(ni) else { return };
        let Some((_, p)) = run.items.get(run.item).and_then(|i| i.parts.get(run.part)) else {
            run.phase = Phase::Done;
            return;
        };
        let (n, bang) = (p.cmds.len(), p.bang);
        if run.killed.is_some() {
            run.code = code;
            run.phase = Phase::Done;
            return;
        }
        if run.stage + 1 < n {
            run.input = Some(core::mem::take(&mut run.pipe));
            run.stage += 1;
            run.phase = Phase::Next;
            return;
        }
        let mut code = if bang { i32::from(code == 0) } else { code };
        if run.interrupted {
            code = 130;
        }
        run.input = None;
        run.pipe.clear();
        run.code = code;
        run.err_line = if code != 0 { Some(run.cur_err.clone().unwrap_or_default()) } else { None };
        if run.interrupted {
            run.phase = Phase::Done;
        } else {
            // the next pipeline of the and-or list that is to run
            let parts = &run.items[run.item].parts;
            let mut part = run.part + 1;
            while part < parts.len() {
                let go = match parts[part].0 {
                    Conn::And => code == 0,
                    Conn::Or => code != 0,
                    Conn::First => true,
                };
                if go {
                    break;
                }
                part += 1;
            }
            if part < parts.len() {
                run.part = part;
                run.stage = 0;
                run.phase = Phase::Next;
            } else {
                run.item += 1;
                run.part = 0;
                run.stage = 0;
                run.phase = if run.item < run.items.len() { Phase::Next } else { Phase::Done };
            }
        }
        if fg && let Some(s) = self.sh_mut(ni) {
            s.status = code;
        }
    }

    /// a file of a redirection gets what its command wrote (/etc/resolv.conf: the resolver)
    fn sh_write_file(&mut self, ni: usize, f: OutFile) -> bool {
        let canon = self.store(ni).and_then(|s| s.norm(&f.abs).ok()).unwrap_or_default();
        let text = String::from_utf8_lossy(&f.data).into_owned();
        if canon == "/etc/resolv.conf" {
            return self.resolv_write(ni, &text, f.append).is_ok();
        }
        if let Err(e) = self.file_put(ni, &f.abs, &f.data, f.append) {
            self.sh_fail(ni, &format!("jfn-sh: {}: {}\n", f.path, e.errno_text()));
            return false;
        }
        true
    }

    /// the run of the context is over
    fn sh_finish(&mut self, ni: usize) {
        match self.sh_bg_ctx(ni) {
            None => {
                if let Some(s) = self.sh_mut(ni) {
                    let run = s.fg.take();
                    s.err_line = run.and_then(|r| r.err_line);
                }
                self.sh_notes(ni);
            }
            Some(no) => {
                let Some(s) = self.sh_mut(ni) else { return };
                let n = s.bg.len();
                let Some(k) = s.bg.iter().position(|j| j.no == no) else { return };
                let mark = if k + 1 == n { '+' } else if k + 2 == n { '-' } else { ' ' };
                let j = &mut s.bg[k];
                let state = match (j.run.killed, j.run.code) {
                    (Some(sig), _) => String::from(signal_word(sig)),
                    (None, 0) => String::from("Done"),
                    (None, c) => format!("Exit {}", c),
                };
                let note = format!("[{}]{}  {:<24}{}", no, mark, state, j.text);
                j.ended = Some(state);
                if !j.waited {
                    s.notes.push(note);
                }
            }
        }
    }

    /// cli_job_done: the console job of the context's run ended
    pub(crate) fn sh_job_ended(&mut self, ni: usize, j: &CliJob) {
        let code = match self.sh_mut(ni).and_then(|s| s.code.take()) {
            Some(c) => c,
            None => self.job_code(j),
        };
        if let Some(run) = self.sh_run_mut(ni)
            && run.phase == Phase::Wait
        {
            run.phase = Phase::Ended(code);
        }
    }

    /// the exit status of a console job that ended by itself: ping 0 with a reply, 1
    /// without, 2 when the name was not found
    fn job_code(&self, j: &CliJob) -> i32 {
        let Some(job) = self.job(j.id) else { return 0 };
        match &job.kind {
            JobKind::Ping(p) if matches!(j.out, JobOut::LinuxPing { .. }) => {
                if p.dst.is_unspecified() {
                    2
                } else {
                    i32::from(p.received == 0)
                }
            }
            JobKind::Ping6(p) if matches!(j.out, JobOut::Ping6 { ios: false, .. } | JobOut::LinuxPing { .. }) => i32::from(p.received == 0),
            JobKind::Trace(t) if matches!(j.out, JobOut::LinuxTrace { .. }) && t.dst.is_unspecified() => 2,
            _ => 0,
        }
    }

    /// Ctrl+C ended the foreground's console job (cli_interrupt): the rest of the pipeline
    /// filters what was collected, the rest of the line goes
    pub(crate) fn sh_interrupted(&mut self, ni: usize, j: Option<&CliJob>) {
        // `fg` / `wait`: the job it waited for gets the signal
        if let Some(CliJob { out: JobOut::Wait { no }, .. }) = j {
            let no = *no;
            self.sh_kill_job(ni, no, 2);
            if let Some(s) = self.sh_mut(ni) {
                s.bg.retain(|x| x.no != no || x.ended.is_none());
            }
        }
        // the terminal ends the line of the ^C (what the command printed on it went elsewhere)
        if self.sh_run(ni).is_some_and(|r| r.out != Sink::Term) {
            self.out_raw(ni, "\n");
        }
        let Some(run) = self.sh_run_mut(ni) else { return };
        run.interrupted = true;
        if run.phase == Phase::Wait {
            run.phase = Phase::Ended(130);
        }
        self.sh_go(ni);
    }

    /// the startup configuration's line is over: whatever still waits goes
    pub(crate) fn sh_abort(&mut self, ni: usize) {
        let jobs: Vec<CliJob> = match self.sh_mut(ni) {
            Some(s) => {
                s.fg = None;
                s.bg.drain(..).filter_map(|j| j.cj).collect()
            }
            None => Vec::new(),
        };
        for j in jobs {
            self.cancel_job(j.id);
            self.cli.jobs.remove(&j.id);
            if let JobOut::Proc { svc, .. } = j.out {
                self.console_kill(ni, svc);
            }
        }
    }

    /// the power went off: a fresh shell when it comes back
    pub(crate) fn sh_power_off(&mut self, ni: usize) {
        // the command in the foreground goes too: the console is free when the power is back
        if let Some(j) = self.cdev_mut(ni).and_then(|c| {
            c.queue.clear();
            c.job.take()
        }) {
            self.cancel_job(j.id);
            self.cli.jobs.remove(&j.id);
            if let JobOut::Proc { svc, .. } = j.out {
                self.console_kill(ni, svc);
            }
        }
        self.sh_abort(ni);
        let style = self.cdev(ni).map(|c| c.style);
        if let (Some(c), Some(st)) = (self.cdev_mut(ni), style) {
            c.sh = Shell::new(st);
        }
    }

    // ------------------------------------------------------------ background jobs

    /// the background job whose console job is `id`
    pub(crate) fn sh_bg_of_job(&self, ni: usize, id: crate::jobs::JobId) -> Option<u32> {
        self.sh(ni)?.bg.iter().find(|j| j.cj.as_ref().is_some_and(|c| c.id == id)).map(|j| j.no)
    }

    /// the background job whose program is service `svc`
    fn sh_bg_of_svc(&self, ni: usize, svc: u32) -> Option<u32> {
        self.sh(ni)?.bg.iter().find(|j| matches!(j.cj, Some(CliJob { out: JobOut::Proc { svc: s, .. }, .. }) if s == svc)).map(|j| j.no)
    }

    /// works on background job `no`: its console job is CliDev::job meanwhile
    fn sh_bg_with(&mut self, ni: usize, no: u32, f: impl FnOnce(&mut Engine)) {
        let Some(c) = self.cdev_mut(ni) else { return };
        let Some(k) = c.sh.bg.iter().position(|j| j.no == no) else { return };
        let cj = c.sh.bg[k].cj.take();
        let fg = core::mem::replace(&mut c.job, cj);
        let saved = self.cli.sh_ctx.replace((ni, no));
        f(self);
        self.cli.sh_ctx = saved;
        let Some(c) = self.cdev_mut(ni) else { return };
        let back = core::mem::replace(&mut c.job, fg);
        match c.sh.bg.iter_mut().find(|j| j.no == no) {
            Some(j) => j.cj = back,
            None => {
                if let Some(b) = back {
                    self.cancel_job(b.id);
                    self.cli.jobs.remove(&b.id);
                }
            }
        }
    }

    /// an event of background job `no` (`f` with its console job in place), then its run
    /// goes on; a job that ended goes (its first output is out), one waited for ends the
    /// foreground's wait
    pub(crate) fn sh_bg_event(&mut self, ni: usize, no: u32, f: impl FnOnce(&mut Engine)) {
        self.sh_bg_with(ni, no, |e| {
            f(e);
            e.sh_go(ni);
        });
        self.sh_bg_sweep(ni);
    }

    /// background jobs that ended go; the foreground's fg / wait learns of it
    fn sh_bg_sweep(&mut self, ni: usize) {
        let waiting = match self.cdev(ni).and_then(|c| c.job.as_ref()) {
            Some(CliJob { out: JobOut::Wait { no }, .. }) => Some(*no),
            _ => None,
        };
        let Some(s) = self.sh_mut(ni) else { return };
        let mut done_code = None;
        if let Some(w) = waiting
            && let Some(j) = s.bg.iter().find(|j| j.no == w && j.ended.is_some())
        {
            done_code = Some(match j.run.killed {
                Some(sig) => 128 + i32::from(sig),
                None => j.run.code,
            });
        }
        s.bg.retain(|j| j.ended.is_none() || j.run.hold.is_some());
        if let Some(c) = done_code {
            self.sh_set_code(ni, c);
            self.cli_job_done(ni);
        }
        self.cli_state(ni);
    }

    /// a wake-up of background job `no` (Ev::Cli with BG_WAKE)
    pub(crate) fn sh_bg_wake(&mut self, ni: usize, no: u32) {
        self.sh_bg_event(ni, no, |e| {
            if e.cdev(ni).is_some_and(|c| c.job.is_some()) {
                e.job_timer(ni);
            }
        });
    }

    /// the and-or list the foreground run is at goes into the background
    fn sh_bg_start(&mut self, ni: usize) {
        let now = self.now;
        let Some(s) = self.sh_mut(ni) else { return };
        let Some(run) = s.fg.as_mut() else { return };
        let Some(mut item) = run.items.get(run.item).cloned() else { return };
        run.item += 1;
        run.part = 0;
        run.stage = 0;
        run.phase = if run.item < run.items.len() { Phase::Next } else { Phase::Done };
        item.bg = false;
        let no = s.bg.iter().map(|j| j.no).max().unwrap_or(0) + 1;
        let text = item.text.clone();
        let mut r = Run::new(alloc::vec![item]);
        r.hold = Some(String::new());
        s.bg.push(BgJob { no, pid: 0, text, run: r, cj: None, start: now, ended: None, waited: false });
        s.status = 0;
        self.sh_bg_with(ni, no, |e| e.sh_go(ni));
        // its PID: its program's, else a number of its own
        let svc = self.sh(ni).and_then(|s| s.bg.iter().find(|j| j.no == no)).and_then(|j| match j.cj {
            Some(CliJob { out: JobOut::Proc { svc, .. }, .. }) => Some(svc),
            _ => None,
        });
        let id = match svc {
            Some(x) => x,
            None => self.os_mut(ni).map_or(0, |o| o.take_id()),
        };
        let pid = 100 + id;
        let held = match self.sh_mut(ni) {
            Some(s) => {
                s.last_bg = Some(pid);
                match s.bg.iter_mut().find(|j| j.no == no) {
                    Some(j) => {
                        j.pid = pid;
                        j.run.hold.take().unwrap_or_default()
                    }
                    None => String::new(),
                }
            }
            None => String::new(),
        };
        self.out_raw(ni, &format!("[{}] {}\n{}", no, pid, held));
        self.sh_bg_sweep(ni);
    }

    /// ends background job `no` with a signal (kill)
    fn sh_kill_job(&mut self, ni: usize, no: u32, sig: u8) {
        if sig == 0 {
            return;
        }
        self.sh_bg_event(ni, no, |e| {
            if let Some(j) = e.cdev_mut(ni).and_then(|c| c.job.take()) {
                e.cancel_job(j.id);
                e.cli.jobs.remove(&j.id);
                if let JobOut::Proc { svc, .. } = j.out {
                    e.console_kill(ni, svc);
                }
            }
            if let Some(run) = e.sh_run_mut(ni) {
                run.killed = Some(sig);
                run.busy = false;
                run.phase = Phase::Ended(128 + i32::from(sig));
            }
        });
    }

    /// OsApi::print / print_err: a console program's output goes where its job's goes
    pub(crate) fn prog_print(&mut self, ni: usize, svc: u32, text: &str, err: bool) {
        // a program of another session prints there (cli/ses.rs)
        if let Some(s) = self.ses_of_prog(ni, svc) {
            self.in_ses(ni, s, |e| e.prog_print(ni, svc, text, err));
            return;
        }
        let saved = self.cli.sh_ctx;
        if let Some(no) = self.sh_bg_of_svc(ni, svc) {
            self.cli.sh_ctx = Some((ni, no));
        }
        if err {
            self.err_out(ni, text);
        } else {
            self.out(ni, text);
        }
        self.cli.sh_ctx = saved;
    }

    /// OsApi::exit: the console (or the background job) of the program goes on at `at`
    pub(crate) fn prog_wake(&mut self, ni: usize, svc: u32, at: Ns) {
        // a program of another session wakes that session (cli/ses.rs)
        if let Some(s) = self.ses_of_prog(ni, svc) {
            self.in_ses(ni, s, |e| e.prog_wake(ni, svc, at));
            return;
        }
        match self.sh_bg_of_svc(ni, svc) {
            Some(no) => self.cli_sched(ni, at, BG_WAKE | no),
            None => self.cli_wake_at(ni, at),
        }
    }

    // ------------------------------------------------------------ the commands of processes

    /// a job by "%N", "%%", "%+", "%-", "%TEXT" or its PID
    fn sh_job_by(&self, ni: usize, spec: &str) -> Option<u32> {
        let s = self.sh(ni)?;
        let live: Vec<&BgJob> = s.bg.iter().filter(|j| j.ended.is_none()).collect();
        match spec {
            "%" | "%%" | "%+" => live.last().map(|j| j.no),
            "%-" => live.len().checked_sub(2).map(|k| live[k].no),
            x if x.starts_with('%') => {
                let r = &x[1..];
                match r.parse::<u32>() {
                    Ok(n) => live.iter().find(|j| j.no == n).map(|j| j.no),
                    Err(_) => live.iter().rev().find(|j| j.text.starts_with(r)).map(|j| j.no),
                }
            }
            x => {
                let pid: u32 = x.parse().ok()?;
                live.iter().find(|j| j.pid == pid || matches!(j.cj, Some(CliJob { out: JobOut::Proc { svc, .. }, .. }) if 100 + svc == pid)).map(|j| j.no)
            }
        }
    }

    /// jobs [-l] [-p]
    pub(crate) fn sh_jobs(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut long, mut pids) = (false, false);
        for a in args {
            match *a {
                "-l" => long = true,
                "-p" => pids = true,
                o => return self.sh_err(ni, &format!("jfn-sh: jobs: {}: invalid option\njobs: usage: jobs [-lp] [jobspec ...]\n", o)),
            }
        }
        let Some(s) = self.sh(ni) else { return Ok(()) };
        let live: Vec<&BgJob> = s.bg.iter().filter(|j| j.ended.is_none()).collect();
        let mut out = String::new();
        for (k, j) in live.iter().enumerate() {
            let mark = if k + 1 == live.len() { '+' } else if k + 2 == live.len() { '-' } else { ' ' };
            if pids {
                out.push_str(&format!("{}\n", j.pid));
            } else if long {
                out.push_str(&format!("[{}]{} {} {:<24}{} &\n", j.no, mark, j.pid, "Running", j.text));
            } else {
                out.push_str(&format!("[{}]{}  {:<24}{} &\n", j.no, mark, "Running", j.text));
            }
        }
        self.out(ni, &out);
        Ok(())
    }

    /// fg [%N] and wait [%N | PID]: the console waits for a background job (its output goes
    /// to the terminal as before; Ctrl+C ends it)
    pub(crate) fn sh_fg(&mut self, ni: usize, args: &[&str], wait: bool) -> Result<(), String> {
        let name = if wait { "wait" } else { "fg" };
        if self.sh_bg_ctx(ni).is_some() {
            return self.sh_err(ni, &format!("jfn-sh: {}: no job control\n", name));
        }
        let spec = args.first().copied().unwrap_or("%+");
        let Some(no) = self.sh_job_by(ni, spec) else {
            if wait && args.is_empty() {
                return Ok(());
            }
            let what = if spec == "%+" { "current" } else { spec };
            self.sh_set_code(ni, if wait { 127 } else { 1 });
            return self.sh_err(ni, &format!("jfn-sh: {}: {}: no such job\n", name, what));
        };
        if !wait {
            let text = self.sh(ni).and_then(|s| s.bg.iter().find(|j| j.no == no)).map(|j| j.text.clone()).unwrap_or_default();
            self.out(ni, &format!("{}\n", text));
        }
        if let Some(j) = self.sh_mut(ni).and_then(|s| s.bg.iter_mut().find(|j| j.no == no)) {
            j.waited = true;
        }
        self.next_job += 1;
        let id = self.next_job;
        let start = self.now;
        if let Some(c) = self.cdev_mut(ni) {
            c.job = Some(CliJob { id, out: JobOut::Wait { no }, start });
        }
        Ok(())
    }

    /// kill [-SIG | -s SIG | -n N] PID|%N ..., kill -l
    pub(crate) fn sh_kill(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        const USAGE: &str = "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]\n";
        let mut sig = 15u8;
        let mut k = 0;
        if args.first() == Some(&"-l") || args.first() == Some(&"-L") {
            let mut s = String::new();
            for (n, name) in SIGNALS.iter().filter(|x| x.0 != 0) {
                s.push_str(&format!("{:2}) SIG{}\n", n, name));
            }
            self.out(ni, &s);
            return Ok(());
        }
        while let Some(a) = args.get(k) {
            if *a == "-s" || *a == "-n" {
                let v = args.get(k + 1).copied().unwrap_or("");
                match signal_of(v) {
                    Some(x) => sig = x,
                    None => {
                        self.sh_set_code(ni, 1);
                        return self.sh_err(ni, &format!("jfn-sh: kill: {}: invalid signal specification\n", v));
                    }
                }
                k += 2;
                continue;
            }
            if let Some(n) = a.strip_prefix('-').filter(|n| !n.is_empty()) {
                match signal_of(n) {
                    Some(x) => sig = x,
                    None => return self.sh_err(ni, &format!("jfn-sh: kill: {}: invalid signal specification\n", n)),
                }
                k += 1;
                continue;
            }
            break;
        }
        let targets = &args[k..];
        if targets.is_empty() {
            self.sh_set_code(ni, 2);
            return self.sh_err(ni, USAGE);
        }
        let mut failed = None;
        for t in targets {
            if let Some(no) = self.sh_job_by(ni, t) {
                self.sh_kill_job(ni, no, sig);
                continue;
            }
            let pid: Option<u32> = t.parse().ok();
            let msg = match pid {
                None if t.starts_with('%') => format!("jfn-sh: kill: {}: no such job\n", t),
                None => format!("jfn-sh: kill: {}: arguments must be process or job IDs\n", t),
                // init and the shell take no signal from here
                Some(1) | Some(SHELL_PID) => continue,
                Some(p) if p > 100 && self.os(ni).is_some_and(|o| o.svcs.iter().any(|s| s.running && 100 + s.id == p)) => format!("jfn-sh: kill: ({}) - Operation not permitted\n", p),
                Some(p) => format!("jfn-sh: kill: ({}) - No such process\n", p),
            };
            self.err_out(ni, &msg);
            failed.get_or_insert(msg);
        }
        match failed {
            Some(m) => Err(String::from(m.trim_end())),
            None => Ok(()),
        }
    }

    /// ps [aux | -ef ...]: every process of the device with its terminal (the options are taken
    /// as they come; the rows are Engine::processes', procs.rs: init, the shell of every session,
    /// the running services and the background jobs of every session)
    pub(crate) fn sh_ps(&mut self, ni: usize, _args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let here = self.ses_here(ni).map_or_else(|| String::from("tty1"), |i| i.line.clone());
        let mut rows: Vec<(u32, String, u64, String)> = self.processes(dev).into_iter().map(|p| (p.pid, p.tty, p.cpu_ns, p.cmd)).collect();
        let me = rows.iter().map(|r| r.0).max().unwrap_or(SHELL_PID) + 1;
        rows.push((me, here, 0, String::from("ps")));
        rows.sort_by_key(|r| r.0);
        let mut s = String::from("  PID USER     TTY      TIME COMMAND\n");
        for (pid, tty, cpu, cmd) in rows {
            s.push_str(&format!("{:>5} {:<8} {:<8} {:>4} {}\n", pid, "root", tty, cpu_time(cpu), cmd));
        }
        self.out(ni, &s);
        Ok(())
    }

    /// sleep NUMBER[smhd] ...: a console job until the time is up
    pub(crate) fn sh_sleep(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        if args.is_empty() {
            return self.sh_err(ni, "sleep: missing operand\nTry 'sleep --help' for more information.\n");
        }
        if args[0] == "--help" {
            self.out(ni, &super::man::usage("sleep"));
            return Ok(());
        }
        let mut d: Ns = 0;
        for a in args {
            match sh::sleep_ns(a) {
                Some(x) => d = d.saturating_add(x),
                None => return self.sh_err(ni, &format!("sleep: invalid time interval '{}'\nTry 'sleep --help' for more information.\n", a)),
            }
        }
        if d == 0 {
            return Ok(());
        }
        let until = self.now + d;
        self.next_job += 1;
        let id = self.next_job;
        let start = self.now;
        if let Some(c) = self.cdev_mut(ni) {
            c.job = Some(CliJob { id, out: JobOut::Sleep { until }, start });
        }
        self.cli_wake_at(ni, until);
        Ok(())
    }

    /// jfs FILE [ARGS ...]: a script of the device's files as a process
    pub(crate) fn sh_jfs(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let Some(file) = args.first().copied() else {
            self.sh_set_code(ni, 2);
            return self.sh_err(ni, "jfs: welches Skript? Aufruf: jfs DATEI.jfs [ARGUMENTE ...]\n");
        };
        if file == "-h" || file == "--help" {
            self.out(ni, &super::man::usage("jfs"));
            return Ok(());
        }
        // the tests of files (cli/jfstest.rs)
        if file == "--test" {
            return self.sh_jfs_test(ni, &args[1..]);
        }
        let abs = self.sh_abs(ni, file);
        let data = match self.sh_read(ni, &abs) {
            Ok(d) => d,
            Err(e) => return self.sh_err(ni, &format!("jfs: {}: {}\n", file, e)),
        };
        let Ok(source) = core::str::from_utf8(&data) else {
            return self.sh_err(ni, &format!("jfs: {}: das ist kein Text in UTF-8\n", file));
        };
        let list: Vec<agent_core::jmap::Value> = args[1..].iter().map(|a| agent_core::jmap::Value::string(*a)).collect();
        let mut conf = agent_core::jmap::empty_object();
        conf.insert("args", agent_core::jmap::Value::Array(list));
        conf.insert("file", agent_core::jmap::Value::string(&abs));
        let svc = match ScriptService::new_file(&abs, source, ScriptCfg::default(), &conf) {
            Ok(s) => s,
            Err(diags) => {
                let mut s = String::new();
                for d in diags.iter().filter(|d| d.error) {
                    s.push_str(&format!("{}:{}\n", file, d));
                }
                return self.sh_err(ni, &s);
            }
        };
        let base = file.rsplit('/').next().unwrap_or(file);
        let name = String::from(base.strip_suffix(".jfs").unwrap_or(base));
        let name = if name.is_empty() { String::from("jfs") } else { name };
        let p = JfsProc { inner: svc, file: String::from(file), shown: 0, errors: 0, done: false };
        self.run_program(ni, &name, Box::new(p), false)
    }
}

// ---------------------------------------------------------------- jfs as a process

/// a script run from the shell: the service of scriptsvc.rs on the console
#[derive(Clone)]
pub struct JfsProc {
    pub inner: ScriptService,
    /// the file as typed (its errors name it)
    file: String,
    /// output printed so far (ScriptService::run_output)
    shown: u64,
    /// errors shown so far
    errors: usize,
    done: bool,
}

impl JfsProc {
    /// after every call: new output to the terminal, new errors to stderr, the end
    fn after(&mut self, os: &mut OsApi) {
        if self.done {
            return;
        }
        let (text, end) = self.inner.run_output(self.shown);
        if !text.is_empty() {
            let t = String::from(text);
            os.print(&t);
        }
        self.shown = end;
        let errs = self.inner.run_errors();
        let failed = self.inner.state() == ScriptState::Failed;
        if errs.len() > self.errors {
            let mut s = String::new();
            for e in &errs[self.errors..] {
                if e.line > 0 {
                    s.push_str(&format!("{}:{}:{}: Fehler: {}\n", self.file, e.line, e.col, e.msg));
                } else {
                    s.push_str(&format!("{}: Fehler: {}\n", self.file, e.msg));
                }
            }
            os.print_err(&s);
            self.errors = errs.len();
        }
        if failed {
            self.done = true;
            os.exit(1);
            return;
        }
        // done when the CPU time of its last callback is over
        match self.inner.idle_at() {
            Some(t) if t <= os.now() => {
                self.done = true;
                os.exit(0);
            }
            Some(t) => os.timer(K_PROC, t),
            None => {}
        }
    }
}

/// the timer key of JfsProc itself (the script's service uses the keys 0 to 2)
const K_PROC: u32 = 3;

impl Service for JfsProc {
    fn kind(&self) -> &'static str {
        "jfs"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.inner.start(os);
        self.after(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if self.done {
            return;
        }
        if ev != SvcEvent::Timer(K_PROC) {
            self.inner.on(ev, os);
        }
        self.after(os);
    }
    fn stop(&mut self, os: &mut OsApi) {
        self.inner.stop(os);
    }
    fn metrics(&self) -> Vec<Metric> {
        self.inner.metrics()
    }
    fn status(&self) -> String {
        self.inner.status()
    }
    fn renamed(&mut self, name: &str) {
        self.inner.renamed(name);
    }
    fn footprint(&self) -> usize {
        self.inner.footprint()
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
