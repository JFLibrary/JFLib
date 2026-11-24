// cli -- the setup's command line: options that preset the pages, the quiet mode (no window:
// an installation or removal by a script, a package manager or a deployment tool) and the log
//
//   Opts::parse(args) -> (Opts, problem)   problem: the first option the setup does not know or
//                                       one without its value (only the quiet mode refuses it;
//                                       a window ignores it, ui has options of its own)
//   quiet(opts, problem) -> exit code   install / uninstall without a window
//   Log                                 --log FILE: what the setup did, line by line ("error: "
//                                       before a failure; the setup with administrator rights
//                                       reports to the one that started it this way)
//
// Exit codes of the quiet mode: 0 done, 1 failed (the reason on stdout / the console and in the
// log), 2 a wrong command line (an unknown option or style), 740 (ERROR_ELEVATION_REQUIRED) an
// installation for all users, or the removal of one, without administrator rights.

use alloc::string::String;
use alloc::vec::Vec;

use ui::i18n::{t, tf};

use crate::shell::{Scope, sh_admin};
use crate::{Inst, PATH_CAP, install, payload, str_cap};

pub(crate) const EXIT_OK: i32 = 0;
pub(crate) const EXIT_FAILED: i32 = 1;
pub(crate) const EXIT_USAGE: i32 = 2;
pub(crate) const EXIT_ELEVATION: i32 = 740;

/// the setup's options (setup.exe [options], uninstall.exe --uninstall [options])
#[derive(Default, Clone)]
pub(crate) struct Opts {
    /// --quiet: no window
    pub(crate) quiet: bool,
    /// --uninstall: remove the installation uninstall.exe belongs to
    pub(crate) uninstall: bool,
    /// --dir PATH: the folder
    pub(crate) dir: Option<String>,
    /// --style ID: the style (quiet: one the setup does not hold is an error)
    pub(crate) style: Option<String>,
    /// --all-users / --current-user
    pub(crate) scope: Option<Scope>,
    /// --no-desktop, --no-start-menu, --no-uninstall-entry: those options off
    pub(crate) no_desktop: bool,
    pub(crate) no_start: bool,
    pub(crate) no_reg: bool,
    /// --launch: start the program when done
    pub(crate) launch: bool,
    /// --log FILE
    pub(crate) log: Option<String>,
}

const VALUES: &[&str] = &["--dir", "--style", "--log"];

/// a problem of the command line: a text of the catalogue and the option it names (translated
/// once the catalogue is loaded)
pub(crate) struct Problem(&'static str, String);

impl Problem {
    pub(crate) fn text(&self) -> String {
        tf(self.0, &[&self.1])
    }
}

impl Opts {
    /// the command line after the program -> the options and the first problem
    pub(crate) fn parse(args: &[String]) -> (Opts, Option<Problem>) {
        let mut o = Opts::default();
        let mut problem = None;
        let mut note = |text: &'static str, a: &str| {
            if problem.is_none() {
                problem = Some(Problem(text, String::from(a)));
            }
        };
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            if VALUES.contains(&a) {
                let Some(v) = args.get(i + 1) else {
                    note("Die Option „{}“ braucht einen Wert.", a);
                    break;
                };
                match a {
                    "--dir" => o.dir = Some(str_cap(v, PATH_CAP)),
                    "--style" => o.style = Some(v.clone()),
                    _ => o.log = Some(str_cap(v, PATH_CAP)),
                }
                i += 2;
                continue;
            }
            match a {
                "--quiet" => o.quiet = true,
                "--uninstall" => o.uninstall = true,
                "--all-users" => o.scope = Some(Scope::Machine),
                "--current-user" => o.scope = Some(Scope::User),
                "--no-desktop" => o.no_desktop = true,
                "--no-start-menu" => o.no_start = true,
                "--no-uninstall-entry" => o.no_reg = true,
                "--launch" => o.launch = true,
                // the language of the texts (ui and i18n read it themselves)
                "--lang" => i += 1,
                _ => {
                    note("Unbekannte Option „{}“.", a);
                    // (a window: ui's own options, the value after one of them too)
                    if plat::VALUE_OPTS.contains(&a) {
                        i += 1;
                    }
                }
            }
            i += 1;
        }
        (o, problem)
    }
}

/// --log FILE: the lines so far, written again after each one (nothing without --log)
#[derive(Default)]
pub(crate) struct Log {
    path: Option<String>,
    text: String,
}

impl Log {
    pub(crate) fn new(path: Option<String>) -> Log {
        Log { path, text: String::new() }
    }

    /// add a line (and write the file)
    pub(crate) fn line(&mut self, s: &str) {
        let Some(p) = &self.path else { return };
        self.text.push_str(s);
        self.text.push_str("\r\n");
        plat::write_file(p, self.text.as_bytes());
    }
}

/// a result for the user: stdout, or the console the setup was started from, and the log
fn report(st: &mut Inst, text: &str) {
    plat::assoc::console_print(&(String::from(text) + "\n"));
    st.log.line(text);
}

/// a failure: reported (in the log after "error: ") -> code
fn fail(st: &mut Inst, code: i32, text: &str) -> i32 {
    plat::assoc::console_print(&(String::from(text) + "\n"));
    st.log.line(&(String::from("error: ") + text));
    code
}

/// the last failure of a log another setup wrote (its "error: " line)
pub(crate) fn log_error(text: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(text);
    s.lines().rev().find_map(|l| l.strip_prefix("error: ")).map(|e| str_cap(e, 512))
}

/// the quiet mode: install, or (--uninstall) remove, without a window -> the exit code
pub(crate) fn quiet(opts: Opts, problem: Option<Problem>) -> i32 {
    crate::catalog_boot(true);
    let mut st = Inst { log: Log::new(opts.log.clone()), ..Inst::default() };
    if let Some(p) = problem {
        return fail(&mut st, EXIT_USAGE, &p.text());
    }
    if opts.uninstall {
        if install::un_load(&mut st).is_err() {
            let e = st.err.clone();
            return fail(&mut st, EXIT_FAILED, &e);
        }
        if st.scope == Scope::Machine && !sh_admin() {
            return fail(&mut st, EXIT_ELEVATION, t("Diese Installation ist für alle Benutzer: Zum Entfernen sind Administratorrechte nötig."));
        }
        st.log.line(&alloc::format!("uninstall: {} in {}", st.app, st.dir));
        install::un_run(&mut st);
        let d = st.disp.clone();
        report(&mut st, &tf("{} wurde entfernt.", &[&d]));
        return EXIT_OK;
    }
    if payload::pl_load(&mut st).is_err() {
        let e = st.err.clone();
        return fail(&mut st, EXIT_FAILED, &e);
    }
    if let Some(want) = &opts.style {
        if payload::sty_find(&st, want).is_none() {
            let have: Vec<&str> = st.sty.iter().map(|s| st.ents[s.ent].name.as_str()).collect();
            let text = tf("Unbekannter Stil „{}“; das Setup enthält: {}.", &[want, &have.join(", ")]);
            return fail(&mut st, EXIT_USAGE, &text);
        }
    }
    st.apply(&opts);
    if st.scope == Scope::Machine && !sh_admin() {
        return fail(&mut st, EXIT_ELEVATION, t("Die Installation für alle Benutzer braucht Administratorrechte: Starten Sie das Setup als Administrator."));
    }
    st.log_start();
    if install::inst_run(&mut st).is_err() {
        let e = st.err.clone();
        return fail(&mut st, EXIT_FAILED, &e);
    }
    if opts.launch {
        install::inst_launch(&st);
    }
    let (d, dir) = (st.disp.clone(), st.dir.clone());
    report(&mut st, &tf("{} wurde in „{}“ installiert.", &[&d, &dir]));
    EXIT_OK
}
