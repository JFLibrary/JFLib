//! jflib-installer -- the setup of apps built with JFLib: choose the look (one exe per style in
//! the payload, previews of each), who it is for, folder and shortcuts, install; with
//! --uninstall the same program removes an installation again (it is copied as uninstall.exe)
//! [apps/installer/src/main.s]
//!
//! ```text
//! setup.exe [--dir PATH] [--style ID] [--all-users | --current-user] [--no-desktop]
//!           [--no-start-menu] [--no-uninstall-entry] [--launch] [--quiet] [--log FILE] [--lang CODE]
//!                                          install (tools/package.py, cargo jflib package append the payload)
//! uninstall.exe --uninstall [--quiet] [--log FILE]   remove (reads install.ini next to it)
//! ```
//!
//! The program is `installer::main!();` (the library is named `installer`) in a `#![no_std]`
//! `#![no_main]` crate whose build script is jflib-build's (apps/installer here; the stub
//! `cargo jflib package` makes).
//! Pages: style -> options -> work -> done (error page on failures); uninstall: ask -> done.
//! A payload with one style has no style page.  The texts are in the user's language
//! (setup.tsv: German, English, French, Spanish, Italian, Dutch, Polish, Arabic, Hebrew; else
//! English), the styles' names and descriptions too when the manifest has them.
//! For all users (Program Files, HKLM) the setup without administrator rights starts itself
//! again with them (the UAC prompt; quiet, waited for) and goes on when that one is done.
//! --quiet installs or removes without a window (cli.rs: exit codes, --log).
//! A web browser (manifest browser=1) gets the option "Als Standardbrowser festlegen" on the
//! options page, checked as in Firefox's setup: "Fertigstellen" then opens Windows' Settings at
//! the program's page of the default apps (plat::assoc::browser_request; Windows lets no
//! program take the default itself).  Test mode and script runs only print it.
//! Tests: ASMINST_SHELL_DIR redirects desktop / start menu / registry (shell.rs);
//! tools/setup_check.py.
//! Windows only (apps/installer/TARGETS).
//
//   lib.rs      the pages (in_frame), the commands (in_cmd), the state (the asm's .bss)
//   cli.rs      the command line, the quiet mode, the log
//   payload.rs  the payload of the own exe: trailer, entry table, manifest, previews
//   install.rs  install / launch / uninstall, install.ini
//   shell.rs    known folders, shortcuts, the uninstall key, processes (plat::inst) + test mode
//   setup.tsv   the texts in every language (i18n::Catalog)
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod assoc;
mod cli;
mod install;
mod payload;
mod shell;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Surface;
use ui::i18n::{self, t, tf};
use ui::*;

use cli::{Log, Opts};
use shell::{Scope, sh_admin, sh_elevate, sh_program_files};

pub(crate) use plat::inst::PATH_CAP;

const ID_ROOT: u32 = 100;
const ID_HEAD: u32 = 101;
const ID_BODY: u32 = 102;
const ID_FOOT: u32 = 103;
const ID_RADIO: u32 = 110; // + style index
const ID_PREVIEW: u32 = 130;
const ID_DIR: u32 = 140;
const ID_DESK: u32 = 141;
const ID_START: u32 = 142;
const ID_REG: u32 = 143;
const ID_LAUNCH: u32 = 144;
const ID_PROG: u32 = 145;
const ID_BROWSER: u32 = 146;
const ID_SCOPE_USER: u32 = 147;
const ID_SCOPE_ALL: u32 = 148;
const ID_CANCEL: u32 = 150;
const ID_BACK: u32 = 151;
const ID_NEXT: u32 = 152;
const ID_STYLES: u32 = 160;
const ID_STYLELIST: u32 = 161;

const CMD_NEXT: u32 = 1;
const CMD_BACK: u32 = 2;
const CMD_INSTALL: u32 = 3;
const CMD_DOINST: u32 = 4;
const CMD_FINISH: u32 = 5;
const CMD_QUIT: u32 = 6;
const CMD_UNINST: u32 = 7;
const CMD_DOUNINST: u32 = 8;
const CMD_ERRBACK: u32 = 9;

// pages (inst.inc PG_*)
const PG_STYLE: u32 = 0;
const PG_OPTS: u32 = 1;
const PG_WORK: u32 = 2;
const PG_DONE: u32 = 3;
const PG_ERR: u32 = 4;
const PG_UASK: u32 = 5;
const PG_UDONE: u32 = 6;

const FMT_CAP: usize = 256; // fmt_buf(g_buf, 256, ...)

const DESC: AppDesc = AppDesc { title: "Setup", flags: 0, w: 820, h: 600, min_w: 820, min_h: 600, ..AppDesc::NONE };

/// str_copy(dst, s, cap): s cut to cap - 1 bytes (never inside a UTF-8 sequence)
pub(crate) fn str_cap(s: &str, cap: usize) -> String {
    if s.len() < cap {
        return String::from(s);
    }
    let mut n = cap - 1;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    String::from(&s[..n])
}

/// one style of the payload (the asm's STY record)
pub(crate) struct Sty {
    /// index of its PK_STYLE entry
    pub(crate) ent: usize,
    /// the decoded preview (STY_SF / STY_HAVE)
    pub(crate) sf: Option<Rc<Surface>>,
    /// its name in the language of the pages (manifest label_<id>.<lang>, else the entry's label)
    pub(crate) label: String,
    /// manifest desc_<id>.<lang>, else desc_<id>
    pub(crate) desc: String,
}

/// the setup's state (the .bss of main.s, payload.s, install.s)
#[derive(Default)]
pub(crate) struct Inst {
    /// the command line
    pub(crate) opts: Opts,
    /// the own exe (g_self, g_selfn) and its payload (g_payoff, g_paysz)
    pub(crate) self_: Vec<u8>,
    pub(crate) payoff: usize,
    pub(crate) paysz: usize,
    /// the entry table
    pub(crate) ents: Vec<payload::Entry>,
    /// the manifest text (g_man)
    pub(crate) man: Option<Vec<u8>>,
    pub(crate) sty: Vec<Sty>,
    pub(crate) err: String,
    /// the page "Zurück" leads to from the error page (a refused UAC prompt: nothing happened)
    pub(crate) err_back: Option<u32>,
    pub(crate) app: String,
    pub(crate) disp: String,
    pub(crate) ver: String,
    pub(crate) pub_: String,
    pub(crate) defsty: String,
    pub(crate) dir: String,
    pub(crate) selfpath: String,
    pub(crate) exepath: String,
    pub(crate) sel: i32,
    pub(crate) page: u32,
    pub(crate) uninst: bool,
    /// for the current user or for all users (the options page's choice: scope_sel 0 / 1)
    pub(crate) scope: Scope,
    pub(crate) scope_sel: i32,
    pub(crate) opt_desktop: i32,
    pub(crate) opt_start: i32,
    pub(crate) opt_reg: i32,
    pub(crate) opt_launch: i32,
    /// (browser=1) make it the default browser after installing (Settings opens on Finish)
    pub(crate) opt_browser: i32,
    /// the shortcuts made (in_lnk_desk, in_lnk_start; uninstall: from install.ini)
    pub(crate) lnk_desk: String,
    pub(crate) lnk_start: String,
    /// uninstall: install.ini files=
    pub(crate) files: String,
    /// the manifest's file types and URL schemes (PACKAGE filetypes= / schemes=)
    pub(crate) types: String,
    pub(crate) schemes: String,
    /// the manifest's browser=1: registered as a web browser too (Default Programs)
    pub(crate) browser: bool,
    /// uninstall: install.ini assoc= (what the file types and schemes added)
    pub(crate) assoc: String,
    /// the manifest's app_id (PACKAGE, tools/pkgmeta.py): the program's AppUserModelID on its
    /// shortcuts, pins and ProgIDs ("" = none: an older payload)
    pub(crate) appid: String,
    /// --log FILE
    pub(crate) log: Log,
}

/// fmt_buf(g_buf, 256, f, a): the catalogue text f with its {} replaced by a, cut to 255 bytes
fn fmt1(f: &'static str, a: &str) -> String {
    str_cap(&tf(f, &[a]), FMT_CAP)
}

/// the setup's texts for the whole process (setup.tsv); quiet: the user's languages from --lang
/// or the system first (a window: ui has read them)
pub(crate) fn catalog_boot(quiet: bool) {
    if quiet {
        i18n::use_system();
    }
    i18n::set_catalog(i18n::Catalog::from_tsv(include_str!("setup.tsv")));
}

impl Inst {
    /// the default folder of a scope: %LOCALAPPDATA%\Programs\<display> for the user, Program
    /// Files\<display> for all users
    fn default_dir(&self, scope: Scope) -> String {
        let base = match scope {
            Scope::User => {
                let local = plat::env("LOCALAPPDATA").filter(|v| !v.is_empty()).map(|v| str_cap(&v, PATH_CAP));
                local.unwrap_or_else(|| String::from("C:")) + "\\Programs"
            }
            Scope::Machine => sh_program_files(),
        };
        str_cap(&(str_cap(&base, PATH_CAP) + "\\" + &self.disp), PATH_CAP)
    }

    /// this app is installed in dir (its install.ini names it)
    fn installed_in(&self, dir: &str) -> bool {
        plat::read_file(&format!("{dir}\\install.ini")).and_then(|ini| payload::kv_get(&ini, "app", 64)).is_some_and(|a| a == self.app)
    }

    /// the choices before the pages (and of the quiet mode): the command line's, else the
    /// defaults.  Who it is for: --all-users / --current-user, else all users when the app is
    /// installed in Program Files and not for the user (running the setup again changes the
    /// style of that installation)
    pub(crate) fn apply(&mut self, o: &Opts) {
        self.opt_desktop = !o.no_desktop as i32;
        self.opt_start = !o.no_start as i32;
        self.opt_reg = !o.no_reg as i32;
        self.opt_launch = o.launch as i32;
        self.opt_browser = 1;
        self.scope = o.scope.unwrap_or_else(|| {
            let machine = o.dir.is_none() && self.installed_in(&self.default_dir(Scope::Machine))
                && !self.installed_in(&self.default_dir(Scope::User));
            if machine { Scope::Machine } else { Scope::User }
        });
        self.scope_sel = (self.scope == Scope::Machine) as i32;
        self.dir = match &o.dir {
            Some(d) => str_cap(d, PATH_CAP),
            None => self.default_dir(self.scope),
        };
        // style: --style, else the manifest's default, else the first
        self.sel = 0;
        let want = o.style.clone().unwrap_or_else(|| self.defsty.clone());
        if let Some(k) = payload::sty_find(self, &want) {
            self.sel = k as i32;
        }
    }

    /// the options page switched who it is for: the folder follows unless the user typed one
    fn set_scope(&mut self, scope: Scope) {
        if self.dir == self.default_dir(self.scope) {
            self.dir = self.default_dir(scope);
        }
        self.scope = scope;
    }

    /// (--log) what is installed where
    pub(crate) fn log_start(&mut self) {
        let style = self.ents[self.sty[install::sel_index(self)].ent].name.clone();
        let who = if self.scope == Scope::Machine { "all users" } else { "the current user" };
        let line = format!("install: {} {} (style {style}) for {who} into {}", self.app, self.ver, self.dir);
        self.log.line(&line);
    }

    /// a temporary file for the log of the setup with administrator rights
    fn elevated_log(&self) -> String {
        let tmp = plat::temp_dir().unwrap_or_else(|| self.dir.clone());
        str_cap(&format!("{}\\{}-setup-{}.log", tmp.trim_end_matches('\\'), self.app, if self.uninst { "remove" } else { "install" }), PATH_CAP)
    }

    /// run the setup again with administrator rights and the choices made (quiet), wait -> Ok,
    /// or Err with st.err (a refused prompt: err_back = the page to return to)
    fn elevated(&mut self, mut args: Vec<String>, back: u32) -> Result<(), ()> {
        let log = self.elevated_log();
        plat::delete(&log);
        args.extend(["--quiet", "--lang"].map(String::from));
        args.push(i18n::language_code());
        args.push(String::from("--log"));
        args.push(log.clone());
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        let r = sh_elevate(&self.selfpath, &a);
        let text = plat::read_file(&log).unwrap_or_default();
        plat::delete(&log);
        for l in String::from_utf8_lossy(&text).lines() {
            self.log.line(l);
        }
        match r {
            Ok(0) => Ok(()),
            Ok(code) => {
                self.err = cli::log_error(&text).unwrap_or_else(|| tf("Das Setup mit Administratorrechten ist fehlgeschlagen (Code {}).", &[&format!("{code}")]));
                Err(())
            }
            Err(()) => {
                self.err = String::from(t("Dafür sind Administratorrechte nötig. Sie wurden nicht erteilt."));
                self.err_back = Some(back);
                Err(())
            }
        }
    }

    /// install as chosen: here, or for all users without administrator rights through the
    /// setup started again with them
    fn install(&mut self) -> Result<(), ()> {
        if self.scope == Scope::Machine && !sh_admin() {
            let style = self.ents[self.sty[install::sel_index(self)].ent].name.clone();
            let mut args: Vec<String> = ["--all-users", "--dir", &self.dir, "--style", &style].map(String::from).into();
            for (off, flag) in [(self.opt_desktop, "--no-desktop"), (self.opt_start, "--no-start-menu"), (self.opt_reg, "--no-uninstall-entry")] {
                if off == 0 {
                    args.push(String::from(flag));
                }
            }
            self.elevated(args, PG_OPTS)?;
            install::inst_read_back(self);
            return Ok(());
        }
        self.log_start();
        let r = install::inst_run(self);
        let line = if r.is_ok() { String::from("done") } else { format!("error: {}", self.err) };
        self.log.line(&line);
        r
    }

    /// remove the installation: here, or (for all users, without administrator rights) through
    /// uninstall.exe started again with them
    fn uninstall(&mut self) -> Result<(), ()> {
        if self.scope == Scope::Machine && !sh_admin() {
            return self.elevated(alloc::vec![String::from("--uninstall")], PG_UASK);
        }
        let line = format!("uninstall: {} in {}", self.app, self.dir);
        self.log.line(&line);
        install::un_run(self);
        self.log.line("done");
        Ok(())
    }
}

impl App for Inst {
    // in_init
    fn init(&mut self, ui: &mut Ui) -> bool {
        catalog_boot(false);
        self.log = Log::new(self.opts.log.clone());
        self.uninst = self.opts.uninstall;
        if self.uninst {
            self.page = if install::un_load(self).is_ok() { PG_UASK } else { PG_ERR };
        } else if payload::pl_load(self).is_err() {
            self.page = PG_ERR;
        } else {
            let o = self.opts.clone();
            self.apply(&o);
            // a payload with one style: nothing to choose
            self.page = if self.sty.len() > 1 { PG_STYLE } else { PG_OPTS };
        }
        // window title "<display> – Setup" / "– Deinstallation"
        let f = if self.uninst { "{} – Deinstallation" } else { "{} – Setup" };
        let d = if self.disp.is_empty() { t("Setup") } else { &self.disp };
        ui.set_title(&fmt1(f, d));
        true
    }

    // in_frame
    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.box_begin(ID_ROOT, BOX_V | PAD_24, 14);
        self.fr_head(ui);
        ui.separator();
        ui.next_size(-1, -1);
        ui.box_begin(ID_BODY, BOX_V, 10);
        match self.page {
            PG_STYLE => self.fr_style(ui),
            PG_OPTS => self.fr_opts(ui),
            PG_WORK => self.fr_work(ui),
            PG_DONE => self.fr_done(ui),
            PG_UASK => self.fr_uask(ui),
            PG_UDONE => self.fr_udone(ui),
            _ => self.fr_error(ui),
        }
        ui.box_end();
        ui.separator();
        self.fr_foot(ui);
        ui.box_end();
    }

    // in_cmd
    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        match id {
            CMD_NEXT => self.page = PG_OPTS,
            CMD_BACK => self.page = PG_STYLE,
            CMD_INSTALL => {
                self.page = PG_WORK;
                ui.cmd_run(CMD_DOINST, 0); // after the progress page was painted
            }
            CMD_DOINST => {
                self.page = if self.install().is_ok() { PG_DONE } else { PG_ERR };
            }
            CMD_FINISH => {
                if self.opt_launch != 0 {
                    install::inst_launch(self);
                }
                // after the program, so that Settings comes to the front
                if self.browser && self.opt_browser != 0 {
                    install::inst_default_browser(self, ui.headless());
                }
                ui.quit();
            }
            CMD_QUIT => ui.quit(),
            CMD_UNINST => {
                self.page = PG_WORK;
                ui.cmd_run(CMD_DOUNINST, 0);
            }
            CMD_DOUNINST => {
                self.page = if self.uninstall().is_ok() { PG_UDONE } else { PG_ERR };
            }
            CMD_ERRBACK => {
                self.page = self.err_back.take().unwrap_or(PG_ERR);
            }
            UIC_ESCAPE => {
                if self.page != PG_WORK {
                    ui.quit();
                }
            }
            UIC_CLOSE => return self.page == PG_WORK, // keep the window only while working
            _ => return false,
        }
        true
    }
}

impl Inst {
    /// program icon (the setup's glyph), title and version line  [fr_head]
    fn fr_head(&mut self, ui: &mut Ui) {
        ui.box_begin(ID_HEAD, BOX_H | BOX_XCENTER, 14);
        ui.icon(if self.uninst { IC_DELETE } else { IC_DOWNLOAD }, 32);
        ui.box_begin(0, BOX_V, 2);
        let f = if self.uninst { "{} deinstallieren" } else { "{} installieren" };
        let d = if self.disp.is_empty() { t("Programm") } else { &self.disp };
        ui.text(TS_TITLE, &fmt1(f, d));
        if !self.ver.is_empty() {
            let line = str_cap(&tf("Version {}   ·   {}", &[&self.ver, &self.pub_]), FMT_CAP);
            ui.text(TS_BODY | TX_SECONDARY, &line);
        }
        ui.box_end();
        ui.box_end();
    }

    /// style list (radio + description) at the left, preview of the selection right  [fr_style]
    fn fr_style(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY | TX_WRAP, t("Wählen Sie das Aussehen. Der Stil wird fest in das Programm eingebaut; um ihn zu wechseln, führen Sie das Setup später einfach erneut aus."));
        ui.next_size(-1, -1);
        ui.box_begin(ID_STYLES, BOX_H, 20);
        // the list
        ui.next_size(-1, 0);
        ui.box_begin(ID_STYLELIST, BOX_V, 4);
        for (i, s) in self.sty.iter().enumerate().take(payload::STY_MAX) {
            ui.radio(ID_RADIO + i as u32, &s.label, &mut self.sel, i as i32);
            if !s.desc.is_empty() {
                ui_spacer_h(ui, 26);
                ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &s.desc);
                ui.box_end();
            }
            ui.spacer(6);
        }
        ui.box_end();
        // preview
        let k = self.sel as u32 as usize;
        if k < self.sty.len() && k < payload::STY_MAX {
            if let Some(sf) = &self.sty[k].sf {
                ui.image(ID_PREVIEW, sf, 0);
            }
        }
        ui.box_end();
    }

    /// who it is for, folder and options  [fr_opts]
    fn fr_opts(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY_STRONG, t("Installieren für"));
        let before = self.scope_sel;
        ui.radio(ID_SCOPE_USER, t("Nur für mich (ohne Administratorrechte)"), &mut self.scope_sel, 0);
        ui.radio(ID_SCOPE_ALL, t("Für alle Benutzer dieses Computers (mit Administratorrechten)"), &mut self.scope_sel, 1);
        if self.scope_sel != before {
            self.set_scope(if self.scope_sel == 1 { Scope::Machine } else { Scope::User });
        }
        ui.spacer(8);
        ui.text(TS_BODY_STRONG, t("Zielordner"));
        ui.next_size(-1, 0);
        ui.textbox(ID_DIR, Some(t("Ordner")), &mut self.dir, PATH_CAP as u32);
        ui.spacer(8);
        ui.text(TS_BODY_STRONG, t("Verknüpfungen"));
        ui.checkbox(ID_DESK, t("Verknüpfung auf dem Desktop erstellen"), &mut self.opt_desktop);
        ui.checkbox(ID_START, t("Zum Startmenü hinzufügen"), &mut self.opt_start);
        ui.checkbox(ID_REG, t("In „Installierte Apps“ eintragen (zum Deinstallieren)"), &mut self.opt_reg);
    }

    /// progress while installing or removing  [fr_work]
    fn fr_work(&mut self, ui: &mut Ui) {
        let text = if self.uninst { fmt1("{} wird entfernt …", &self.disp) } else { String::from(t("Die Dateien werden installiert …")) };
        ui.text(TS_BODY, &text);
        ui.next_size(-1, 0);
        ui.progress(ID_PROG, 1, 2);
    }

    /// finished; what "Fertigstellen" does: start the program, (a web browser) Settings at its
    /// page of the default apps  [fr_done]
    fn fr_done(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, &fmt1("{} wurde erfolgreich installiert.", &self.disp));
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &fmt1("Ordner: {}", &self.dir));
        if self.scope == Scope::Machine {
            fd_check(ui, t("Für alle Benutzer dieses Computers"));
        }
        if !self.lnk_desk.is_empty() {
            fd_check(ui, t("Verknüpfung auf dem Desktop"));
        }
        if !self.lnk_start.is_empty() {
            fd_check(ui, t("Eintrag im Startmenü"));
        }
        ui.spacer(12);
        let l = fmt1("{} jetzt starten", &self.disp);
        ui.checkbox(ID_LAUNCH, &l, &mut self.opt_launch);
        if self.browser {
            let b = fmt1("Als Standardbrowser festlegen", &self.disp);
            ui.checkbox(ID_BROWSER, &b, &mut self.opt_browser);
        }
    }

    /// what went wrong  [fr_error]
    fn fr_error(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, t("Das hat nicht geklappt"));
        ui.text(TS_BODY | TX_WRAP, &self.err);
    }

    /// uninstall: confirm  [fr_uask]
    fn fr_uask(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE | TX_WRAP, &fmt1("Möchten Sie {} von diesem Computer entfernen?", &self.disp));
        ui.text(TS_BODY | TX_WRAP, t("Das Programm, seine Verknüpfungen und der Eintrag in „Installierte Apps“ werden gelöscht. Ihre Einstellungen und Ihre eigenen Dateien bleiben erhalten."));
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &fmt1("Ordner: {}", &self.dir));
    }

    /// uninstall finished  [fr_udone]
    fn fr_udone(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, &fmt1("{} wurde entfernt.", &self.disp));
    }

    /// buttons of the page (right-aligned; the style orders nothing here)  [fr_foot]
    fn fr_foot(&mut self, ui: &mut Ui) {
        ui.next_size(-1, 0);
        ui.box_begin(ID_FOOT, BOX_H | BOX_XCENTER, 8);
        ui.spacer(-1);
        match self.page {
            PG_STYLE => {
                ff_cancel(ui);
                ff_button_accent(ui, ID_NEXT, t("Weiter"), CMD_NEXT);
            }
            PG_OPTS => {
                ff_cancel(ui);
                if self.sty.len() > 1 {
                    ff_button(ui, ID_BACK, t("Zurück"), CMD_BACK);
                }
                ff_button_accent(ui, ID_NEXT, t("Installieren"), CMD_INSTALL);
            }
            PG_WORK => {}
            PG_DONE => ff_button_accent(ui, ID_NEXT, t("Fertigstellen"), CMD_FINISH),
            PG_UASK => {
                ff_cancel(ui);
                ff_button_accent(ui, ID_NEXT, t("Deinstallieren"), CMD_UNINST);
            }
            // error / uninstall done: close (a refused UAC prompt: back to the page before)
            _ => {
                if self.page == PG_ERR && self.err_back.is_some() {
                    ff_button(ui, ID_BACK, t("Zurück"), CMD_ERRBACK);
                }
                ff_button_accent(ui, ID_NEXT, t("Schließen"), CMD_QUIT);
            }
        }
        ui.box_end();
    }
}

/// open a horizontal row starting with an indent of dip (closed by the caller with box_end);
/// the row's text fills the rest  [ui_spacer_h]
fn ui_spacer_h(ui: &mut Ui, dip: i32) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H, 0);
    ui.spacer(dip);
    ui.next_size(-1, 0);
}

/// a row: check mark icon + text  [fd_check]
fn fd_check(ui: &mut Ui, text: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.icon(IC_CHECK_MARK, 16);
    ui.text(TS_BODY, text);
    ui.box_end();
}

fn ff_cancel(ui: &mut Ui) {
    ff_button(ui, ID_CANCEL, t("Abbrechen"), CMD_QUIT);
}

/// queue cmd when clicked  [ff_button]
fn ff_button(ui: &mut Ui, id: u32, label: &str, cmd: u32) {
    if ui.button(id, label) & EV_ACT != 0 {
        ui.cmd_run(cmd, 0);
    }
}

/// [ff_button_accent]
fn ff_button_accent(ui: &mut Ui, id: u32, label: &str, cmd: u32) {
    if ui.accent_button(id, label) & EV_ACT != 0 {
        ui.cmd_run(cmd, 0);
    }
}

/// the setup: its window and pages until it ends, or (--quiet) its work without a window ->
/// the exit code
pub fn launch() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    let (opts, problem) = Opts::parse(&args);
    if opts.quiet {
        return cli::quiet(opts, problem);
    }
    ui::launch(&DESC, Inst { opts, ..Inst::default() }, &style_winui::STYLE)
}

#[doc(hidden)]
pub use ui as __ui;

/// the setup stub's entry point, in a `#![no_std] #![no_main]` program built with jflib-build
#[macro_export]
macro_rules! main {
    () => {
        $crate::__ui::main!($crate::launch());
    };
}
