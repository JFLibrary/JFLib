// installer -- setup for apps built with jfguiLib: choose the look (one exe per style in the
// payload, previews of each), folder and shortcuts, install; with --uninstall the same program
// removes an installation again (it is copied as uninstall.exe)  [apps/installer/src/main.s]
//
//   setup.exe [--dir PATH] [--style ID]      install (tools/package.py appends the payload)
//   uninstall.exe --uninstall                remove (reads install.ini next to it)
//
// Pages: style -> options -> work -> done (error page on failures); uninstall: ask -> done.
// Tests: ASMINST_SHELL_DIR redirects desktop / start menu / registry (shell.rs).
// Windows only (apps/installer/TARGETS).
//
//   main.rs     the pages (in_frame), the commands (in_cmd), the state (the asm's .bss)
//   payload.rs  the payload of the own exe: trailer, entry table, manifest, previews
//   install.rs  install / launch / uninstall, install.ini
//   shell.rs    known folders, shortcuts, the uninstall key, processes (plat::inst) + test mode
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod assoc;
mod install;
mod payload;
mod shell;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Surface;
use ui::*;

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
    /// manifest desc_<id> (192 bytes)
    pub(crate) desc: String,
}

/// the setup's state (the .bss of main.s, payload.s, install.s)
#[derive(Default)]
pub(crate) struct Inst {
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
    pub(crate) opt_desktop: i32,
    pub(crate) opt_start: i32,
    pub(crate) opt_reg: i32,
    pub(crate) opt_launch: i32,
    /// the shortcuts made (in_lnk_desk, in_lnk_start; uninstall: from install.ini)
    pub(crate) lnk_desk: String,
    pub(crate) lnk_start: String,
    /// uninstall: install.ini files=
    pub(crate) files: String,
    /// the manifest's file types and URL schemes (PACKAGE filetypes= / schemes=)
    pub(crate) types: String,
    pub(crate) schemes: String,
    /// uninstall: install.ini assoc= (what the file types and schemes added)
    pub(crate) assoc: String,
}

/// fmt_buf(g_buf, 256, f, a): f with its %s replaced by a, cut to 255 bytes
fn fmt1(f: &str, a: &str) -> String {
    str_cap(&f.replacen("%s", a, 1), FMT_CAP)
}

/// arg_flag(name): name appears on the command line (after the program)
fn arg_flag(name: &str) -> bool {
    plat::args().iter().skip(1).any(|a| a == name)
}

impl App for Inst {
    // in_init
    fn init(&mut self, ui: &mut Ui) -> bool {
        self.opt_desktop = 1;
        self.opt_start = 1;
        self.opt_reg = 1;
        self.opt_launch = 0;
        self.uninst = arg_flag("--uninstall");
        if self.uninst {
            self.page = if install::un_load(self).is_ok() { PG_UASK } else { PG_ERR };
        } else if payload::pl_load(self).is_err() {
            self.page = PG_ERR;
        } else {
            self.page = PG_STYLE;
            // folder: --dir, else %LOCALAPPDATA%\Programs\<display>
            self.dir = match plat::arg_value("--dir") {
                Some(d) => str_cap(&d, PATH_CAP),
                None => {
                    let base = plat::env("LOCALAPPDATA").filter(|v| !v.is_empty()).map(|v| str_cap(&v, PATH_CAP));
                    let d = base.unwrap_or_else(|| String::from("C:")) + "\\Programs\\";
                    str_cap(&(str_cap(&d, PATH_CAP) + &self.disp), PATH_CAP)
                }
            };
            // style: --style, else the manifest's default, else the first
            self.sel = 0;
            let want = plat::arg_value("--style").unwrap_or_else(|| self.defsty.clone());
            if let Some(k) = payload::sty_find(self, &want) {
                self.sel = k as i32;
            }
        }
        // window title "<display> – Setup" / "– Deinstallation"
        let f = if self.uninst { "%s – Deinstallation" } else { "%s – Setup" };
        let d = if self.disp.is_empty() { "Setup" } else { &self.disp };
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
            PG_WORK => fr_work(ui),
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
                self.page = if install::inst_run(self).is_ok() { PG_DONE } else { PG_ERR };
            }
            CMD_FINISH => {
                if self.opt_launch != 0 {
                    install::inst_launch(self);
                }
                ui.quit();
            }
            CMD_QUIT => ui.quit(),
            CMD_UNINST => {
                self.page = PG_WORK;
                ui.cmd_run(CMD_DOUNINST, 0);
            }
            CMD_DOUNINST => {
                install::un_run(self);
                self.page = PG_UDONE;
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
        let f = if self.uninst { "%s deinstallieren" } else { "%s installieren" };
        let d = if self.disp.is_empty() { "Programm" } else { &self.disp };
        ui.text(TS_TITLE, &fmt1(f, d));
        if !self.ver.is_empty() {
            let t = str_cap(&format!("Version {}   ·   {}", self.ver, self.pub_), FMT_CAP);
            ui.text(TS_BODY | TX_SECONDARY, &t);
        }
        ui.box_end();
        ui.box_end();
    }

    /// style list (radio + description) at the left, preview of the selection right  [fr_style]
    fn fr_style(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY | TX_WRAP, "Wählen Sie das Aussehen. Der Stil wird fest in das Programm eingebaut; um ihn zu wechseln, führen Sie das Setup später einfach erneut aus.");
        ui.next_size(-1, -1);
        ui.box_begin(ID_STYLES, BOX_H, 20);
        // the list
        ui.next_size(-1, 0);
        ui.box_begin(ID_STYLELIST, BOX_V, 4);
        for (i, s) in self.sty.iter().enumerate().take(payload::STY_MAX) {
            ui.radio(ID_RADIO + i as u32, &self.ents[s.ent].label, &mut self.sel, i as i32);
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

    /// folder and options  [fr_opts]
    fn fr_opts(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY_STRONG, "Zielordner");
        ui.next_size(-1, 0);
        ui.textbox(ID_DIR, Some("Ordner"), &mut self.dir, PATH_CAP as u32);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, "Installation nur für Ihr Benutzerkonto – es werden keine Administratorrechte benötigt.");
        ui.spacer(8);
        ui.text(TS_BODY_STRONG, "Verknüpfungen");
        ui.checkbox(ID_DESK, "Verknüpfung auf dem Desktop erstellen", &mut self.opt_desktop);
        ui.checkbox(ID_START, "Zum Startmenü hinzufügen", &mut self.opt_start);
        ui.checkbox(ID_REG, "In „Installierte Apps“ eintragen (zum Deinstallieren)", &mut self.opt_reg);
        let t = fmt1("%s nach der Installation starten", &self.disp);
        ui.checkbox(ID_LAUNCH, &t, &mut self.opt_launch);
    }

    /// finished  [fr_done]
    fn fr_done(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, &fmt1("%s wurde erfolgreich installiert.", &self.disp));
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &fmt1("Ordner: %s", &self.dir));
        if !self.lnk_desk.is_empty() {
            fd_check(ui, "Verknüpfung auf dem Desktop");
        }
        if !self.lnk_start.is_empty() {
            fd_check(ui, "Eintrag im Startmenü");
        }
    }

    /// what went wrong  [fr_error]
    fn fr_error(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, "Das hat nicht geklappt");
        ui.text(TS_BODY | TX_WRAP, &self.err);
    }

    /// uninstall: confirm  [fr_uask]
    fn fr_uask(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE | TX_WRAP, &fmt1("Möchten Sie %s von diesem Computer entfernen?", &self.disp));
        ui.text(TS_BODY | TX_WRAP, "Das Programm, seine Verknüpfungen und der Eintrag in „Installierte Apps“ werden gelöscht. Ihre Einstellungen und Bilder bleiben erhalten.");
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &fmt1("Ordner: %s", &self.dir));
    }

    /// uninstall finished  [fr_udone]
    fn fr_udone(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, &fmt1("%s wurde entfernt.", &self.disp));
    }

    /// buttons of the page (right-aligned; the style orders nothing here)  [fr_foot]
    fn fr_foot(&mut self, ui: &mut Ui) {
        ui.next_size(-1, 0);
        ui.box_begin(ID_FOOT, BOX_H | BOX_XCENTER, 8);
        ui.spacer(-1);
        match self.page {
            PG_STYLE => {
                ff_cancel(ui);
                ff_button_accent(ui, ID_NEXT, "Weiter", CMD_NEXT);
            }
            PG_OPTS => {
                ff_cancel(ui);
                ff_button(ui, ID_BACK, "Zurück", CMD_BACK);
                ff_button_accent(ui, ID_NEXT, "Installieren", CMD_INSTALL);
            }
            PG_WORK => {}
            PG_DONE => ff_button_accent(ui, ID_NEXT, "Fertigstellen", CMD_FINISH),
            PG_UASK => {
                ff_cancel(ui);
                ff_button_accent(ui, ID_NEXT, "Deinstallieren", CMD_UNINST);
            }
            // error / uninstall done: close
            _ => ff_button_accent(ui, ID_NEXT, "Schließen", CMD_QUIT),
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

/// progress while installing  [fr_work]
fn fr_work(ui: &mut Ui) {
    ui.text(TS_BODY, "Die Dateien werden installiert …");
    ui.next_size(-1, 0);
    ui.progress(ID_PROG, 1, 2);
}

/// a row: check mark icon + text  [fd_check]
fn fd_check(ui: &mut Ui, text: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.icon(IC_CHECK_MARK, 16);
    ui.text(TS_BODY, text);
    ui.box_end();
}

fn ff_cancel(ui: &mut Ui) {
    ff_button(ui, ID_CANCEL, "Abbrechen", CMD_QUIT);
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

ui::main!(ui::launch(&DESC, Inst::default(), &style::STYLE));
