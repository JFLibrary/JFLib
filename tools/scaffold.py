#!/usr/bin/env python3
"""scaffold.py -- `python build_rs.py new`: a new app under apps/NAME, or a library crate under lib/NAME

    python build_rs.py new NAME [--kind window|console|tool|service] [--style S] [--title "..."]
    python build_rs.py new --lib NAME [--deps rt,plat] [--title "..."]

App kinds (docs/GUIDE.md section 2):
  window   a desktop window (ui::main!): menu bar and command table, a counter, an info dialog,
           a status bar with the app's and the library's version; TARGETS win linux web
  console  a console program (rt::main!): arguments, --help / --version / --check; win linux
  tool     a window and a command line program that share an app-local crate: src/main.rs (the
           window), cli/main.rs (BINS: NAME-cli), crates/<name>_core (counts lines, words and
           characters); win linux
  service  a background program (rt::main!, plat::service): a work loop, `service install |
           uninstall | start | stop | status`, --once for the test; win linux

Every app gets src/main.rs (and the kind's further sources), TARGETS, STYLE (window and tool), a
PERMISSIONS whose active lines are what the skeleton needs and whose comments list the other
rules, PACKAGE (app_id and homepage from the git remote when there is one, version=0.1.0),
tests/smoke.uis and a README.md.  The app builds and its smoke test passes as generated
(`python build_rs.py test NAME`).  Names: a letter, then letters, digits, '_' or '-' (at most 40);
not an existing app (also not in another case: Windows checkouts), not a library crate, a style
crate or a Rust keyword.

A library crate (--lib): lib/NAME/src/lib.rs (#![no_std], #![forbid(unsafe_code)], a
placeholder function), an entry in LIBS of build_rs.py with --deps (the crates of LIBS it uses,
default none) and in CRATES of tools/gendoc.py, then docs/API.md is regenerated.  Names:
snake_case.  An app uses it by naming it (NAME::...).

apps/scaffoldtest is the output of `new scaffoldtest --kind tool --title "Scaffold-Test"`;
tools/tests/test_devkit.py checks that the generator still writes exactly its sources, and the
suite builds and runs it, so a library change that breaks the skeletons shows up there.
"""
import datetime
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
KINDS = ("window", "console", "tool", "service")
KEYWORDS = set("""as async await break const continue crate dyn else enum extern false fn for gen if impl in
let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where
while abstract become box do final macro override priv typeof unsized virtual yield try union""".split())
RESERVED = {"core", "alloc", "std", "style", "test", "proc_macro", "main", "build", "lib"}
APP_NAME = re.compile(r"[A-Za-z][A-Za-z0-9_-]{0,39}")
LIB_NAME = re.compile(r"[a-z][a-z0-9_]{0,39}")


class ScaffoldError(Exception):
    pass


# ---------------------------------------------------------------- names and git
def crate_name(name):
    """the rustc crate name build_rs.py gives an app (Builder.build_app)"""
    return name.replace("-", "_").lower()


def type_name(name):
    """'my_tool' / 'hello-world' / 'PaintCraft' -> 'MyTool' / 'HelloWorld' / 'PaintCraft'"""
    parts = [p for p in re.split(r"[_-]+", name) if p]
    return "".join(p[:1].upper() + p[1:] for p in parts) or "App"


def check_title(title):
    if not title or len(title) > 80 or any(ord(c) < 0x20 for c in title) or '"' in title or "\\" in title:
        raise ScaffoldError(f"title {title!r}: 1 to 80 characters, no quotes, backslashes or control characters")
    return title


def git_info(root):
    """{'homepage', 'owner', 'publisher'} from the git remote origin and user.name (empty when
    there is none); user names and tokens of an https remote never end up in it"""
    def git(*args):
        try:
            r = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, timeout=10,
                               encoding="utf-8", errors="replace")
            return r.stdout.strip() if r.returncode == 0 else ""
        except (OSError, subprocess.SubprocessError):
            return ""
    info = {"publisher": git("config", "user.name")}
    info.update(parse_remote(git("remote", "get-url", "origin")))
    return info


def parse_remote(url):
    """a git remote -> {'homepage': https URL or '', 'owner': (host, owner) or ''}; the user and
    password of an https remote are dropped"""
    m = (re.match(r"(?:https?|ssh|git)://(?:[^@/]+@)?([^/:@]+)(?::\d+)?/(.+?)(?:\.git)?/?$", url)
         or re.match(r"(?:[^@/:]+@)?([^/:@]+):(?!//)(.+?)(?:\.git)?/?$", url))
    if not m or "/" not in m.group(2) or len(m.group(1)) < 2:     # C:/repos/x is a drive, not a host
        return {"homepage": "", "owner": ""}
    host, path = m.group(1).lower(), m.group(2).strip("/")
    owner = (host, path.split("/")[0]) if host in ("github.com", "gitlab.com", "codeberg.org") else ""
    return {"homepage": f"https://{host}/{path}", "owner": owner}


def app_id_for(name, info):
    """io.github.<owner>.<App> for a GitHub project (the Flathub rule; gitlab / codeberg alike),
    else org.jflib.<App> (tools/pkgmeta.py's default)"""
    part = re.sub(r"[^A-Za-z0-9_]", "_", name)
    if info.get("owner"):
        host, owner = info["owner"]
        prefix = {"github.com": "io.github", "gitlab.com": "io.gitlab", "codeberg.org": "page.codeberg"}[host]
        owner = re.sub(r"[^A-Za-z0-9_]", "_", owner)
        if owner[:1].isdigit():
            owner = "_" + owner
        return f"{prefix}.{owner}.{part}"
    return f"org.jflib.{part}"


def check_app_name(root, name, crates):
    """refuse a name an app cannot have (crates: every library crate name of the build)"""
    if not APP_NAME.fullmatch(name or ""):
        raise ScaffoldError(f"{name!r} is no app name: a letter, then letters, digits, '_' or '-' (at most 40)")
    cn = crate_name(name)
    if cn in KEYWORDS or cn in RESERVED or name in KEYWORDS:
        raise ScaffoldError(f"{name}: a Rust keyword or a reserved name")
    taken = {c.lower() for c in crates}
    if cn in taken or cn.startswith("style_") or f"{cn}_core" in taken:
        raise ScaffoldError(f"{name}: the crate name {cn} is taken by a library crate")
    apps = root / "apps"
    for p in apps.iterdir() if apps.is_dir() else []:
        if p.name.lower() == name.lower():
            raise ScaffoldError(f"apps/{p.name} exists already" + ("" if p.name == name else " (names differ only in case)"))


def check_lib_name(root, name, crates):
    if not LIB_NAME.fullmatch(name or ""):
        raise ScaffoldError(f"{name!r} is no library crate name: snake_case (a-z, 0-9, '_'), starting with a letter")
    if name in crates:
        raise ScaffoldError(f"{name}: the crate exists already")
    if name in KEYWORDS or name in RESERVED or name.startswith("style_"):
        raise ScaffoldError(f"{name}: a Rust keyword or a reserved name")
    if (root / "lib" / name).exists():
        raise ScaffoldError(f"lib/{name} exists already")


def table(rows, indent="//   "):
    """comment lines of (command, text) rows, the texts in one column; a row with an empty
    command continues the text above, a row with an empty text stands alone"""
    w = max((len(c) for c, t in rows if c and t), default=0)
    return "\n".join((indent + c.ljust(w) + "   " + t).rstrip() if t else indent + c for c, t in rows)


def fill(text, values):
    """@KEY@ -> values[KEY]; an unknown key is an error (a typo in a template)"""
    def one(m):
        if m.group(1) not in values:
            raise KeyError(m.group(1))
        return values[m.group(1)]
    return re.sub(r"@([A-Z_]+)@", one, text)


# ---------------------------------------------------------------- app templates
HEAD = """\
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
"""

WINDOW_MAIN = """\
// @APP@ -- @TITLE@
//
// A window app, generated by
//   @CMD@
// Build and start it (Linux: add --target linux), build it and run tests/*.uis on both
// targets, run the test or take a picture without a window:
//   python build_rs.py run @APP@
//   python build_rs.py test @APP@
//   build/win/@APP@.exe --script apps/@APP@/tests/smoke.uis
//   build/win/@APP@.exe --shot build/@APP@.png --scale 150
//
// The app implements ui::App: frame() builds the whole UI on every pass, command() gets the
// commands of the menu bar, the buttons and the shortcuts (the command table CMDS).  Split it
// into modules (src/view.rs, src/model.rs, ...) as it grows; docs/GUIDE.md describes layout,
// widgets, dialogs, files and tests, docs/API.md every function.
@HEAD@
use alloc::format;
use ui::*;

/// the app's own version: version= of apps/@APP@/PACKAGE (the library's is ui::VERSION)
const APP_VERSION: &str = rt::app_version!();

const ID_COUNT: u32 = 100;
const ID_ABOUT: u32 = 101;

const CMD_RESET: u32 = 1;
const CMD_ABOUT: u32 = 2;
const CMD_QUIT: u32 = 3;

static CMDS: [Command; 3] = [
    //           id         label             icon     mods      key           CF_* state
    Command::new(CMD_RESET, "&Zurücksetzen", IC_UNDO, MOD_CTRL, b'R' as u32, 0),
    Command::new(CMD_ABOUT, "&Info …", IC_INFO, 0, 0, 0),
    Command::new(CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0),
];
static M_FILE: [Mi; 3] = [Mi::Cmd(CMD_RESET), Mi::Sep, Mi::Cmd(CMD_QUIT)];
static M_HELP: [Mi; 1] = [Mi::Cmd(CMD_ABOUT)];
static MENUBAR: [Mi; 2] = [Mi::Menu("&Datei", &M_FILE), Mi::Menu("&Hilfe", &M_HELP)];

const DESC: AppDesc = AppDesc {
    title: "@TITLE@",
    flags: AF_RESIZABLE,
    w: 640,
    h: 420,
    min_w: 360,
    min_h: 260,
    cmds: &CMDS,
    menubar: &MENUBAR,
    ..AppDesc::NONE
};

struct @TYPE@ {
    count: u32,
    about: Shared<i32>, // the info dialog is open (the library clears it on Esc / a button)
}

impl Default for @TYPE@ {
    fn default() -> Self {
        @TYPE@ { count: 0, about: Shared::new(0) }
    }
}

impl App for @TYPE@ {
    fn frame(&mut self, ui: &mut Ui) {
        ui.cmd_state(CMD_RESET, if self.count == 0 { CF_DISABLED } else { 0 });
        ui.menubar(None); // the menu bar of DESC
        ui.next_size(0, -1); // the content takes the height between menu and status bar
        ui.box_begin(0, BOX_V | pad(24), 12);
        ui.text(TS_TITLE, "@TITLE@");
        ui.text(TS_BODY | TX_SECONDARY, &format!("Klicks: {}", self.count));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        if ui.accent_button(ID_COUNT, "Klick mich") & EV_ACT != 0 {
            self.count += 1;
        }
        ui.cmd_button(CMD_RESET, 0); // label, icon, shortcut and state from the command table
        ui.box_end();
        ui.box_end();
        ui.statusbar_begin();
        ui.text(TS_CAPTION, &format!("@TITLE@ {} · JFLib {}", APP_VERSION, ui::VERSION));
        ui.statusbar_end();
        // dialogs and windows last: on top of everything else
        let about = self.about.clone();
        if ui.dialog_begin(ID_ABOUT, "Über @TITLE@", &about) {
            ui.text(TS_BODY, &format!("@TITLE@, Version {}", APP_VERSION));
            ui.text(TS_CAPTION | TX_SECONDARY, &format!("Erstellt mit JFLib {}", ui::VERSION));
            ui.dialog_buttons(None, None, Some("Schließen"));
        }
        ui.dialog_end(); // always
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        match id {
            CMD_RESET => self.count = 0,
            CMD_ABOUT => self.about.set(1),
            CMD_QUIT => ui.quit(),
            _ => return false, // not ours: the library's default (UIC_CLOSE closes the window)
        }
        true
    }
}

ui::main!(ui::launch(&DESC, @TYPE@::default(), &style::STYLE));
"""

WINDOW_SMOKE = """\
# @APP@: the smoke test of `build_rs.py new` (build_rs.py test runs every tests/*.uis; the
# script language: docs/GUIDE.md section 12)
expect "Zurücksetzen" disabled
click "Klick mich"
click "Klick mich"
expect text:"Klicks: 2" exists
expect "Zurücksetzen" enabled
key ctrl+r
expect text:"Klicks: 0" exists
click "Hilfe"
click "Info …"
expect dialog:"Über @TITLE@" exists
click "Schließen"
expect dialog:"Über @TITLE@" missing
shot build/@APP@.png
"""

CONSOLE_MAIN = """\
// @APP@ -- @TITLE@
//
// A console program, generated by
//   @CMD@
// Usage:
@USAGE@
// Build and start it (Linux: add --target linux), build it and run tests/*.uis on both targets:
//   python build_rs.py run @APP@ -- Anna Ben
//   python build_rs.py test @APP@
//
// rt::main! installs the app's manifest (PERMISSIONS) and assets (app://) before run(), whose
// value is the exit code.  Write with plat::write_stdout / write_stderr, read the arguments with
// plat::args() / plat::arg_value (docs/GUIDE.md section 16).
@HEAD@
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// the app's own version: version= of apps/@APP@/PACKAGE (the library's is plat::VERSION)
const APP_VERSION: &str = rt::app_version!();

/// the greeting for one name
fn greet(name: &str) -> String {
    format!("Hallo, {}!", name)
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn usage() -> i32 {
    out("Aufruf: @APP@ [NAME ...] | --check | --version | --help\\n");
    0
}

/// the self-check: every case of greet()
fn check() -> i32 {
    let cases = [("Welt", "Hallo, Welt!"), ("Ünal", "Hallo, Ünal!"), ("", "Hallo, !")];
    let mut failed = 0;
    for (name, want) in cases {
        let got = greet(name);
        if got != want {
            out(&format!("FAIL greet({:?}) = {:?}, erwartet {:?}\\n", name, got, want));
            failed += 1;
        }
    }
    out(&format!("@APP@: {} von {} Prüfungen bestanden\\n", cases.len() - failed, cases.len()));
    if failed == 0 { 0 } else { 1 }
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    let mut names = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => return usage(),
            "--version" | "-V" => {
                out(&format!("@APP@ {} (JFLib {})\\n", APP_VERSION, plat::VERSION));
                return 0;
            }
            "--check" => return check(),
            "--script" => i += 1, // build_rs.py test passes the script; a console program ignores it
            a if a.starts_with('-') => {
                plat::write_stderr(format!("@APP@: unbekannte Option {}\\n", a).as_bytes());
                return 2;
            }
            a => names.push(a),
        }
        i += 1;
    }
    if names.is_empty() {
        names.push("Welt");
    }
    for n in names {
        out(&format!("{}\\n", greet(n)));
    }
    0
}

rt::main!(run());
"""

CONSOLE_SMOKE = """\
#!args --check
# @APP@ is a console program: build_rs.py test starts it with the arguments of the first line
# (and --script FILE, which it ignores); exit code 0 = every check passed
"""

TOOL_MAIN = """\
// @APP@ -- @TITLE@
//
// A window and a command line program on one shared crate, generated by
//   @CMD@
// Its parts:
@PARTS@
// Start the window (Linux: add --target linux) or the command line program, build both and
// run tests/*.uis on both targets:
//   python build_rs.py run @APP@
//   python build_rs.py run @APP@ --bin @CLI@ -- FILE
//   python build_rs.py test @APP@
//
// The app implements ui::App: frame() builds the whole UI on every pass, command() gets the
// commands of the menu bar and the shortcuts (the command table CMDS).
@HEAD@
use alloc::format;
use alloc::string::String;
use ui::*;

/// the app's own version: version= of apps/@APP@/PACKAGE (the library's is ui::VERSION)
const APP_VERSION: &str = rt::app_version!();

const ID_TEXT: u32 = 100;
const ID_ABOUT: u32 = 101;

const CMD_CLEAR: u32 = 1;
const CMD_ABOUT: u32 = 2;
const CMD_QUIT: u32 = 3;

static CMDS: [Command; 3] = [
    //           id         label        icon      mods      key           CF_* state
    Command::new(CMD_CLEAR, "&Leeren", IC_DELETE, MOD_CTRL, b'L' as u32, 0),
    Command::new(CMD_ABOUT, "&Info …", IC_INFO, 0, 0, 0),
    Command::new(CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0),
];
static M_FILE: [Mi; 3] = [Mi::Cmd(CMD_CLEAR), Mi::Sep, Mi::Cmd(CMD_QUIT)];
static M_HELP: [Mi; 1] = [Mi::Cmd(CMD_ABOUT)];
static MENUBAR: [Mi; 2] = [Mi::Menu("&Datei", &M_FILE), Mi::Menu("&Hilfe", &M_HELP)];

const DESC: AppDesc = AppDesc {
    title: "@TITLE@",
    flags: AF_RESIZABLE,
    w: 640,
    h: 480,
    min_w: 360,
    min_h: 300,
    cmds: &CMDS,
    menubar: &MENUBAR,
    ..AppDesc::NONE
};

struct @TYPE@ {
    text: String,
    about: Shared<i32>, // the info dialog is open (the library clears it on Esc / a button)
}

impl Default for @TYPE@ {
    fn default() -> Self {
        @TYPE@ { text: String::new(), about: Shared::new(0) }
    }
}

impl App for @TYPE@ {
    fn frame(&mut self, ui: &mut Ui) {
        ui.cmd_state(CMD_CLEAR, if self.text.is_empty() { CF_DISABLED } else { 0 });
        ui.menubar(None); // the menu bar of DESC
        ui.next_size(0, -1); // the content takes the height between menu and status bar
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, "@TITLE@");
        ui.next_size(-1, 240); // the full width, 240 DIP high (it scrolls inside)
        ui.textbox(ID_TEXT, Some("Text eingeben oder einfügen"), &mut self.text, 1 << 20 | TB_MULTI | TB_WRAP);
        let counts = @CORE@::count(&self.text);
        ui.text(TS_BODY, &@CORE@::summary(&counts));
        ui.box_end();
        ui.statusbar_begin();
        ui.text(TS_CAPTION, &format!("@TITLE@ {} · JFLib {}", APP_VERSION, ui::VERSION));
        ui.statusbar_end();
        // dialogs and windows last: on top of everything else
        let about = self.about.clone();
        if ui.dialog_begin(ID_ABOUT, "Über @TITLE@", &about) {
            ui.text(TS_BODY, &format!("@TITLE@, Version {}", APP_VERSION));
            ui.text(TS_CAPTION | TX_SECONDARY, &format!("Erstellt mit JFLib {}", ui::VERSION));
            ui.dialog_buttons(None, None, Some("Schließen"));
        }
        ui.dialog_end(); // always
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        match id {
            CMD_CLEAR => self.text.clear(),
            CMD_ABOUT => self.about.set(1),
            CMD_QUIT => ui.quit(),
            _ => return false, // not ours: the library's default (UIC_CLOSE closes the window)
        }
        true
    }
}

ui::main!(ui::launch(&DESC, @TYPE@::default(), &style::STYLE));
"""

TOOL_CLI = """\
// @CLI@ -- the command line program of @APP@
//
// The lines, words and characters of files, counted by the same crate as the window
// (crates/@CORE@); a program of apps/@APP@/BINS.
@CLI_USAGE@
// Build and start it:
//   python build_rs.py run @APP@ --bin @CLI@ -- apps/@APP@/README.md
//
// The files named on the command line are the user's: `fs.user` in apps/@APP@/PERMISSIONS
// lets the program read them and nothing else (docs/PERMISSIONS.md).
@HEAD@
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use @CORE@::Counts;

/// the app's own version: version= of apps/@APP@/PACKAGE (the library's is plat::VERSION)
const APP_VERSION: &str = rt::app_version!();

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        out(&format!("@CLI@ {} (JFLib {})\\n", APP_VERSION, plat::VERSION));
        return 0;
    }
    let files: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if files.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        out("Aufruf: @CLI@ DATEI ... | --version | --help\\n");
        return if files.is_empty() { 2 } else { 0 };
    }
    let (mut sum, mut failed) = (Counts::default(), 0);
    for f in &files {
        let Some(bytes) = plat::read_file(f) else {
            plat::write_stderr(format!("@CLI@: {} kann nicht gelesen werden\\n", f).as_bytes());
            failed += 1;
            continue;
        };
        let c = @CORE@::count(&String::from_utf8_lossy(&bytes));
        out(&format!("{}: {}\\n", f, @CORE@::summary(&c)));
        sum.add(&c);
    }
    if files.len() > 1 {
        out(&format!("zusammen: {}\\n", @CORE@::summary(&sum)));
    }
    if failed == 0 { 0 } else { 1 }
}

rt::main!(run());
"""

TOOL_CORE = """\
// @CORE@ -- what the window and the command line program of @APP@ share
//
// An app-local crate, built and linted like the library crates (docs/GUIDE.md section 16):
//   count(text) -> Counts        lines, words and characters
//   summary(&Counts) -> String   "2 Zeilen, 3 Wörter, 14 Zeichen"
//   Counts::add                  sums
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

/// what count() finds in a text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// lines: the line breaks, plus one for a last line without a break
    pub lines: usize,
    /// words: runs of characters that are no white space
    pub words: usize,
    /// characters (Unicode scalar values)
    pub chars: usize,
}

impl Counts {
    /// add the counts of another text
    pub fn add(&mut self, o: &Counts) {
        self.lines += o.lines;
        self.words += o.words;
        self.chars += o.chars;
    }
}

/// the lines, words and characters of a text
pub fn count(text: &str) -> Counts {
    let mut c = Counts::default();
    let mut in_word = false;
    for ch in text.chars() {
        c.chars += 1;
        if ch == '\\n' {
            c.lines += 1;
        }
        let space = ch.is_whitespace();
        if !space && !in_word {
            c.words += 1;
        }
        in_word = !space;
    }
    if !text.is_empty() && !text.ends_with('\\n') {
        c.lines += 1;
    }
    c
}

/// the counts as one German line
pub fn summary(c: &Counts) -> String {
    let n = |k: usize, one: &str, many: &str| format!("{} {}", k, if k == 1 { one } else { many });
    format!("{}, {}, {}", n(c.lines, "Zeile", "Zeilen"), n(c.words, "Wort", "Wörter"), n(c.chars, "Zeichen", "Zeichen"))
}
"""

TOOL_BINS = """\
# @APP@'s further programs (build_rs.py, section "app-bins"): NAME ENTRY cli|gui
@CLI@   cli/main.rs   cli    # the counts for files on the command line
"""

TOOL_SMOKE = """\
# @APP@: the smoke test of `build_rs.py new` (build_rs.py test runs every tests/*.uis; the
# script language: docs/GUIDE.md section 12)
expect text:"0 Zeilen, 0 Wörter, 0 Zeichen" exists
expect "Leeren" missing
click #100
type "eins zwei\\ndrei"
expect text:"2 Zeilen, 3 Wörter, 14 Zeichen" exists
key ctrl+a
type "Wort"
expect text:"1 Zeile, 1 Wort, 4 Zeichen" exists
click "Datei"
click "Leeren"
expect text:"0 Zeilen, 0 Wörter, 0 Zeichen" exists
click "Hilfe"
click "Info …"
expect dialog:"Über @TITLE@" exists
key esc
expect dialog:"Über @TITLE@" missing
shot build/@APP@.png
"""

SERVICE_MAIN = """\
// @APP@ -- @TITLE@
//
// A background service, generated by
//   @CMD@
// Usage:
@USAGE@
// Build and start it (Linux: add --target linux), build it and run tests/*.uis on both targets:
//   python build_rs.py run @APP@ -- --once
//   python build_rs.py test @APP@
//
// The service manager starts `@APP@ run --service`: plat::service::run_service reports to the
// Windows service control manager, and stop_requested() turns true when it asks the program to
// stop (Linux: systemd sends SIGTERM).  one_round() is the work: here it writes
// data://status.txt and a line to logs://@SERVICE@.log.  The app's own folders (config://,
// data://, cache://, logs://) need no permission; everything else does (apps/@APP@/PERMISSIONS).
@HEAD@
use alloc::format;
use alloc::string::String;
use alloc::vec;

/// the app's own version: version= of apps/@APP@/PACKAGE (the library's is plat::VERSION)
const APP_VERSION: &str = rt::app_version!();
/// the name of the Windows service and of the systemd unit
const SERVICE: &str = "@SERVICE@";
/// the seconds between two rounds
const PERIOD_S: u32 = @PERIOD@;

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn fail(e: &str) -> i32 {
    plat::write_stderr(format!("@APP@: {}\\n", e).as_bytes());
    1
}

fn usage() -> i32 {
    out(concat!(
        "Aufruf: @APP@ [run] | --once | service install|uninstall|start|stop|status | --version | --help\\n",
        "  run       arbeitet, bis der Dienst angehalten wird (Strg+C, SIGTERM)\\n",
        "  --once    eine Runde, dann Ende\\n",
        "  service   der Systemdienst @SERVICE@ (als Administrator / root)\\n",
    ));
    0
}

/// one round of the work: the round's number and time (UTC) into data://status.txt
fn one_round(round: u64) -> Result<(), String> {
    let now = datetime::Timestamp { secs: plat::unix_time_ns() / 1_000_000_000, nanos: 0 };
    let line = format!("@APP@ {}: Runde {} um {}\\n", APP_VERSION, round, datetime::rfc3339(now));
    if !plat::write_file("data://status.txt", line.as_bytes()) {
        return Err(String::from("data://status.txt kann nicht geschrieben werden"));
    }
    plat::log_info(SERVICE, line.trim_end());
    Ok(())
}

/// the work loop: a round, then PERIOD_S seconds of waiting that a stop request ends early
fn serve(once: bool) -> i32 {
    plat::log_file(Some("logs://@SERVICE@.log"), 1 << 20, 3);
    plat::log_info(SERVICE, &format!("gestartet: @APP@ {} (JFLib {})", APP_VERSION, plat::VERSION));
    let mut round = 0;
    loop {
        round += 1;
        if let Err(e) = one_round(round) {
            plat::log_error(SERVICE, &e);
            return fail(&e);
        }
        if once {
            break;
        }
        for _ in 0..PERIOD_S {
            if plat::service::stop_requested() {
                plat::log_info(SERVICE, "angehalten");
                return 0;
            }
            plat::sleep_ms(1000);
        }
    }
    out(&format!("@APP@: {} Runde(n), Stand in data://status.txt\\n", round));
    0
}

fn serve_forever() -> i32 {
    serve(false)
}

/// @APP@ service ...
fn service_cmd(sub: &str) -> i32 {
    let r = match sub {
        "install" => {
            let Some(exe) = plat::current_exe() else { return fail("der eigene Programmpfad ist unbekannt") };
            let args = vec![String::from("run"), String::from("--service")];
            let spec = plat::service::ServiceSpec {
                name: SERVICE,
                display: "@TITLE@",
                description: "@TITLE@ (@APP@, erstellt mit JFLib)",
                exe: &exe,
                args: &args,
            };
            plat::service::install(&spec)
                .and_then(|_| plat::service::start(SERVICE))
                .map(|_| format!("Dienst {} eingerichtet und gestartet", SERVICE))
        }
        "uninstall" => plat::service::uninstall(SERVICE).map(|_| format!("Dienst {} entfernt", SERVICE)),
        "start" => plat::service::start(SERVICE).map(|_| format!("Dienst {} gestartet", SERVICE)),
        "stop" => plat::service::stop(SERVICE).map(|_| format!("Dienst {} angehalten", SERVICE)),
        "status" => Ok(format!("Dienst {}: {}", SERVICE, plat::service::state(SERVICE).text())),
        _ => return usage(),
    };
    match r {
        Ok(m) => {
            out(&format!("{}\\n", m));
            0
        }
        Err(e) => fail(&e),
    }
}

fn run() -> i32 {
    let args = plat::args();
    let rest: alloc::vec::Vec<&str> = args.iter().skip(1).map(|s| s.as_str()).collect();
    if rest.iter().any(|a| *a == "--help" || *a == "-h") {
        return usage();
    }
    if rest.iter().any(|a| *a == "--version" || *a == "-V") {
        out(&format!("@APP@ {} (JFLib {})\\n", APP_VERSION, plat::VERSION));
        return 0;
    }
    if rest.first() == Some(&"service") {
        return service_cmd(rest.get(1).copied().unwrap_or("status"));
    }
    if rest.contains(&"--service") {
        return plat::service::run_service(SERVICE, serve_forever);
    }
    serve(rest.contains(&"--once"))
}

rt::main!(run());
"""

SERVICE_SMOKE = """\
#!args --once
# @APP@ is a background program: build_rs.py test starts it with the arguments of the first line
# (and --script FILE, which it ignores); --once does one round (data://status.txt), exit code 0
"""

PERMS_HEAD = """\
# @APP@: what the program may do (docs/PERMISSIONS.md).  Without this file an app may do
# everything; with it, everything that is not listed is refused (in the library, plugins,
# scripts and agent actions too).  The app's own folders (config://, data://, cache://,
# logs://) and app:// need no line.  The build checks the syntax; the packages follow the file
# (Flatpak finish-args, MSIX capabilities: docs/PACKAGING.md).
"""

PERMS_GUI = PERMS_HEAD + """\
fs.user                              # the files the user picks (file dialog, drops, "Open with")
#
# Remove the # of a rule the app needs:
# fs.read  $DOCUMENTS/**             read below the documents folder (documents://)
# fs.write $DOCUMENTS/Reports/**     write (and read) below a folder
# url      https mailto              open web pages and mail addresses (plat::open_url)
# net.connect api.example.org:443    connections to a server (netio, lib/update)
# print                              print (plat::print)
# secrets                            passwords and tokens in the system's store (plat::secret)
# autostart                          start at login (plat::autostart_set)
# audio.output                       play sound (plat::audio, ui.beep; Flatpak: --socket=pulseaudio)
# ask fs.read $PICTURES/** "Bilder öffnen"   ask the user at the first access instead
# sandbox                            let the OS enforce this manifest too (Landlock+seccomp / mitigations)
"""

PERMS_CONSOLE = PERMS_HEAD + """\
fs.user                              # the files named on the command line
#
# Remove the # of a rule the program needs:
# fs.read  $HOME/**                  read below the home folder
# fs.write $TEMP/@APP@-*/**          write (and read) below a folder
# process  git                       start programs (plat::process::Command)
# net.connect api.example.org:443    connections to a server (netio)
"""

PERMS_SERVICE = PERMS_HEAD + """\
#
# `@APP@ service install | uninstall | start | stop | status` on Linux: the unit file, whether
# systemd runs, systemctl (Windows asks the service control manager, which needs no line)
fs.write /etc/systemd/system/@SERVICE@.service
fs.read  /lib/systemd/system/@SERVICE@.service
fs.read  /run/systemd/system
process  systemctl
#
# Remove the # of a rule the service needs:
# net.listen  127.0.0.1:8080         a local server (netio::http)
# net.connect api.example.org:443    connections to a server
# fs.read  $HOME/**                  read below the home folder
"""

PACKAGE = """\
# apps/@APP@/PACKAGE: the app's identity and packages (docs/PACKAGING.md)
#   python tools/package.py @APP@ --formats all
# version= is the app's own version (MAJOR.MINOR.PATCH, docs/VERSIONING.md); the program reads it
# as rt::app_version!(), the packages and lib/update use it.  The library has its own (lib/VERSION).
@PACKAGE_NOTE@display=@TITLE@
version=0.1.0
publisher=@PUBLISHER@
summary=@PKG_SUMMARY@
description=@TITLE@: @PKG_SUMMARY@. Diese Beschreibung durch eine eigene ersetzen.
generic=@GENERIC@
categories=@CATEGORIES@
deb_name=@DEB@
# Flatpak, AppStream and the MSIX identity: reverse DNS of a domain or project you control
app_id=@APP_ID@
@HOMEPAGE@default_win=@STYLE@
default_linux=@STYLE@
styles=@STYLE@
"""

README = """\
# @TITLE@

@SUMMARY@ Made with `@CMD@` on @DATE@.

## Build, run, test

```sh
python build_rs.py app @APP@                 # build/win/@EXE@ and build/linux/@APP@
@RUN@
python build_rs.py test @APP@                # build it and run tests/*.uis (Windows and Linux)
```

`python build_rs.py test --quick` runs these tests together with every other app's.
docs/GUIDE.md describes the app model, layout, widgets, files and the test scripts.

## Files

| File | What it is |
|---|---|
@FILES@
| `PERMISSIONS` | what the program may do; the commented lines are rules it might need ([PERMISSIONS.md](../../docs/PERMISSIONS.md)) |
| `PACKAGE` | name, version, app id and package settings ([PACKAGING.md](../../docs/PACKAGING.md)) |
| `TARGETS` | the targets it builds for |
| `tests/smoke.uis` | the test `build_rs.py test` runs |

## Versions

`version=` in `PACKAGE` is this app's own version (MAJOR.MINOR.PATCH). The program reads it as
`rt::app_version!()`; the packages and `lib/update` use it. The library has a version of its own
(`lib/VERSION`, `@LIBCONST@`); [VERSIONING.md](../../docs/VERSIONING.md) explains both and
[CHANGELOG.md](../../CHANGELOG.md) lists the library's changes.
"""

KIND_INFO = {
    "window": dict(summary="A desktop window app built with JFLib.", pkg="Ein Programm mit Fenster, erstellt mit JFLib",
                   generic="Anwendung", categories="Utility;",
                   run="python build_rs.py run @APP@                 # build and start (Linux: --target linux)",
                   files=[("src/main.rs", "the app: its window, menus, commands and state"),
                          ("STYLE", "the style it is built with (docs/STYLES.md)")]),
    "console": dict(summary="A console program built with JFLib.", pkg="Ein Kommandozeilenprogramm, erstellt mit JFLib",
                    generic="Kommandozeilenprogramm",
                    categories="Utility;ConsoleOnly;",
                    run="python build_rs.py run @APP@ -- Anna        # build and start (Linux: --target linux)",
                    files=[("src/main.rs", "the program: arguments, output, the self-check")]),
    "tool": dict(summary="A JFLib tool: a window and a command line program on one shared crate.",
                 pkg="Ein Werkzeug mit Fenster und Kommandozeile, erstellt mit JFLib",
                 generic="Werkzeug", categories="Utility;",
                 run="python build_rs.py run @APP@                 # the window (Linux: --target linux)\n"
                     "python build_rs.py run @APP@ --bin @CLI@ -- README.md   # the command line program",
                 files=[("src/main.rs", "the window"),
                        ("cli/main.rs", "the command line program `@CLI@`"),
                        ("crates/@CORE@/lib.rs", "the code both share (an app-local crate)"),
                        ("BINS", "the further programs of the app (here `@CLI@`)"),
                        ("STYLE", "the style the window is built with (docs/STYLES.md)")]),
    "service": dict(summary="A background service built with JFLib.", pkg="Ein Hintergrunddienst, erstellt mit JFLib",
                    generic="Dienst", categories="Utility;ConsoleOnly;",
                    run="python build_rs.py run @APP@ -- --once      # one round (Linux: --target linux)",
                    files=[("src/main.rs", "the service: its work loop and `service install | uninstall | start | stop | status`")]),
}


def app_files(name, kind="window", style="winui", title=None, info=None, date=None):
    """{relative path below apps/NAME: text} of a new app"""
    if kind not in KINDS:
        raise ScaffoldError(f"unknown kind {kind} ({', '.join(KINDS)})")
    title = check_title(title or name)
    info = info or {"homepage": "", "owner": "", "publisher": ""}
    cn = crate_name(name)
    v = {"APP": name, "TITLE": title, "TYPE": type_name(name), "CORE": f"{cn}_core", "CLI": f"{name}-cli",
         "SERVICE": name.lower().replace("_", "-"), "PERIOD": "10", "STYLE": style, "KIND": kind,
         "DATE": date or datetime.date.today().isoformat(), "EXE": f"{name}.exe",
         "LIBCONST": "plat::VERSION" if kind in ("console", "service") else "ui::VERSION",
         "PUBLISHER": info.get("publisher") or "JFLib", "APP_ID": app_id_for(name, info),
         "DEB": re.sub(r"[^a-z0-9.+-]", "-", name.lower()),
         "HOMEPAGE": f"homepage={info['homepage']}\n" if info.get("homepage") else "",
         "PACKAGE_NOTE": "" if kind in ("window", "tool") else
         "# A program without a window: tools/package.py makes desktop packages for window apps; here\n"
         "# version= (rt::app_version!()) and app_id= matter.\n"}
    v["HEAD"] = HEAD
    app, cli, svc = name, v["CLI"], v["SERVICE"]
    v["USAGE"] = table({
        "console": [(f"{app} [NAME ...]", 'greets every NAME (without one: "Welt")'),
                    (f"{app} --check", "the self-check build_rs.py test runs (tests/smoke.uis): exit code 1"),
                    ("", "when a check fails"),
                    (f"{app} --version | --help", "")],
        "service": [(f"{app} [run]", f"work until stopped: a round every {v['PERIOD']} s (Ctrl+C, SIGTERM end it)"),
                    (f"{app} --once", "one round, then the end (the test build_rs.py test runs)"),
                    (f"{app} service install | uninstall | start | stop | status", ""),
                    ("", f"the system service {svc} (Windows: the service control manager,"),
                    ("", "as administrator; Linux: a systemd unit in /etc/systemd/system, as root)"),
                    (f"{app} --version | --help", "")]}.get(kind, []))
    v["PARTS"] = table([("src/main.rs", "this window: text in, counts out (ui::main!)"),
                        ("cli/main.rs", f"{cli}: the same counts for files (apps/{app}/BINS, rt::main!)"),
                        (f"crates/{v['CORE']}/lib.rs", "what both use (an app-local crate, GUIDE.md section 16)")])
    v["CLI_USAGE"] = table([(f"{cli} FILE ...", "one line per file, then the sum (exit code 1 when a file"),
                            ("", "cannot be read)"), (f"{cli} --version | --help", "")])
    v["CMD"] = (f"python build_rs.py new {name}" + ("" if kind == "window" else f" --kind {kind}")
                + ("" if style == "winui" or kind not in ("window", "tool") else f" --style {style}")
                + ("" if title == name else f' --title "{title}"'))
    k = KIND_INFO[kind]
    v.update(SUMMARY=k["summary"], PKG_SUMMARY=k["pkg"], GENERIC=k["generic"], CATEGORIES=k["categories"])
    v["RUN"] = fill(k["run"], v)
    v["FILES"] = "\n".join(f"| `{fill(f, v)}` | {fill(d, v)} |" for f, d in k["files"])
    files = {"PACKAGE": fill(PACKAGE, v), "README.md": fill(README, v)}
    if kind == "window":
        files.update({"src/main.rs": WINDOW_MAIN, "tests/smoke.uis": WINDOW_SMOKE, "PERMISSIONS": PERMS_GUI,
                      "TARGETS": "win linux web\n", "STYLE": style + "\n"})
    elif kind == "console":
        files.update({"src/main.rs": CONSOLE_MAIN, "tests/smoke.uis": CONSOLE_SMOKE, "PERMISSIONS": PERMS_CONSOLE,
                      "TARGETS": "win linux\n"})
    elif kind == "tool":
        files.update({"src/main.rs": TOOL_MAIN, "cli/main.rs": TOOL_CLI, f"crates/{v['CORE']}/lib.rs": TOOL_CORE,
                      "BINS": TOOL_BINS, "tests/smoke.uis": TOOL_SMOKE, "PERMISSIONS": PERMS_GUI.replace(
                          "# the files the user picks (file dialog, drops, \"Open with\")",
                          "# the files the user picks; @CLI@: the files on its command line"),
                      "TARGETS": "win linux\n", "STYLE": style + "\n"})
    else:
        files.update({"src/main.rs": SERVICE_MAIN, "tests/smoke.uis": SERVICE_SMOKE, "PERMISSIONS": PERMS_SERVICE,
                      "TARGETS": "win linux\n"})
    return {p: fill(t, v) for p, t in files.items()}


def write_files(base, files):
    """the files below base with LF line ends -> their paths"""
    out = []
    for rel, text in files.items():
        p = base / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(text.encode("utf-8"))
        out.append(p)
    return out


def create_app(root, name, kind="window", style="winui", title=None, crates=(), styles=("winui",), date=None):
    """apps/NAME of `kind` -> the written paths (ScaffoldError: nothing was written)"""
    check_app_name(root, name, crates)
    if style not in styles:
        raise ScaffoldError(f"unknown style {style} (styles: {', '.join(styles)})")
    files = app_files(name, kind, style, title, git_info(root), date)
    return write_files(root / "apps" / name, files)


# ---------------------------------------------------------------- library crates
LIB_MAIN = """\
// @NAME@ -- @TITLE@
//
// A library crate, generated by `python build_rs.py new --lib @NAME@`:
//   hello(name) -> String   a placeholder: replace it with the crate's API
//
// Registered in LIBS of build_rs.py (its dependencies: @DEPS_TEXT@) and in CRATES of
// tools/gendoc.py, which documents its public items in docs/API.md (rerun
// `python tools/gendoc.py` after a change).  An app or another crate that names it (@NAME@::...)
// gets it linked; its public items follow the library's version rules (docs/VERSIONING.md).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

/// the greeting for `name` (a placeholder for the crate's own API)
pub fn hello(name: &str) -> String {
    format!("Hallo, {}!", name)
}
"""


def register_lib(text, name, deps, title):
    """build_rs.py's text with `name` appended to LIBS (after its dependencies: a valid build
    order)"""
    m = re.search(r"^LIBS = \{\n.*?^\}\n", text, re.M | re.S)
    if not m:
        raise ScaffoldError("build_rs.py: the LIBS table was not found")
    line = f'    "{name}": ("lib/{name}/src/lib.rs", [{", ".join(repr(d).replace(chr(39), chr(34)) for d in deps)}]),  # {title}\n'
    end = m.end() - 2                              # before the closing "}\n"
    return text[:end] + line + text[end:]


def register_gendoc(text, name, title):
    """tools/gendoc.py's text with `name` appended to CRATES"""
    m = re.search(r"^CRATES = \[\n.*?^\]\n", text, re.M | re.S)
    if not m:
        raise ScaffoldError("tools/gendoc.py: the CRATES table was not found")
    end = m.end() - 2
    return text[:end] + f'    ("{name}", "{title}", "full"),\n' + text[end:]


def create_lib(root, name, deps=(), title=None, libs=(), crates=()):
    """lib/NAME, registered in build_rs.py and tools/gendoc.py -> the written paths"""
    check_lib_name(root, name, crates)
    title = check_title(title or "a JFLib library crate")
    bad = [d for d in deps if d not in libs]
    if bad:
        raise ScaffoldError(f"unknown dependencies {', '.join(bad)} (crates of LIBS: {', '.join(libs)})")
    order = [d for d in libs if d in deps]          # LIBS order: a valid build order
    build = root / "build_rs.py"
    gendoc = root / "tools" / "gendoc.py"

    def edit(path, change):
        """change(text with LF) -> new text, written with the file's own line ends"""
        raw = path.read_bytes().decode("utf-8")
        crlf = "\r\n" in raw
        new = change(raw.replace("\r\n", "\n"))
        return lambda: path.write_bytes((new.replace("\n", "\r\n") if crlf else new).encode("utf-8"))
    write_build = edit(build, lambda t: register_lib(t, name, order, title))
    write_gendoc = edit(gendoc, lambda t: register_gendoc(t, name, title))
    v = {"NAME": name, "TITLE": title, "DEPS_TEXT": ", ".join(order) or "none"}
    paths = write_files(root / "lib" / name, {"src/lib.rs": fill(LIB_MAIN, v)})
    write_build()
    write_gendoc()
    return paths + [build, gendoc]
