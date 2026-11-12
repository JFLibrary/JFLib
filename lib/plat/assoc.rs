// assoc -- the app's file types and URL schemes (apps/NAME/PACKAGE filetypes= / schemes=)
// registered for the running program without a setup: developer and portable builds.  The
// setup (apps/installer), the .deb, MSIX and Flatpak declare them themselves (docs/PACKAGING.md).
//
//   package() -> Package                what build_rs.py built in (JFGUI_PACKAGE: display,
//                                       app_id, categories, filetypes, schemes; ui::main! hands
//                                       it over with package_set); without it: the app's name,
//                                       no types
//   Package::parse(text)                that text ("key=value" lines)
//   parse_types(text) -> [FileType]     "ext[,ext..]|mime|description[|own]; ..." -- every
//                                       extension gets the app under "Open with"; "own" types
//                                       also open with it by default
//   parse_schemes(text) -> [scheme]     "scheme scheme ..." (';', ',' or blanks)
//   reg_entries(app, display, exe, types, schemes, app_id) -> [RegValue]   the values below
//                                       HKCU\Software\Classes the setup and assoc_register write
//                                       (app_id: the program's AppUserModelID in its ProgIDs,
//                                       "" = none)
//   package_entries(&Package, exe) -> [RegValue]   all of a package's values: reg_entries and,
//                                       with browser=1, the web browser's registration (below)
//   app_id_boot() -> Option<id>         (ui::main!, Windows) PACKAGE's app id as the process's
//                                       explicit AppUserModelID (taskbar.rs) once the setup
//                                       installed this program (install.ini beside it names the
//                                       id) or --register registered it (the record names it);
//                                       jfgui_AUMID=ID gives one, jfgui_AUMID=off none.  Every
//                                       other program keeps the id Windows derives from its path
//   assoc_conflicts(&Package) -> Result<[Conflict], AssocError>   the defaults of other
//                                       programs a registration would replace
//   assoc_register(&Package, take) -> Result<Registered, AssocError>   register this program;
//                                       take(&Conflict) decides each default of another program
//                                       (false: it stays, the app is only offered under "Open
//                                       with"); a registration of before is undone first
//   assoc_unregister() -> Result<bool, AssocError>   undo exactly what the record lists (false:
//                                       nothing was registered)
//   assoc_registered() -> Option<String>   the program the record names
//   default_browser_status() -> BrowserStatus   (PACKAGE browser=1) is this app the system's web
//                                       browser: Default, NotDefault, Unknown (browser_status(pkg))
//   default_browser_request() -> Result<(), AssocError>   let the user make it the default: the
//                                       system's own page or question (browser_request(pkg))
//   file_uri_path(arg) -> Option<String>   a local file:// URI as a path (Linux desktops hand
//                                       files over as URIs for Exec=... %U)
//   console_print(text)                 stdout, else the console the program was started from
//                                       (Windows programs of the GUI subsystem: --register)
//
// The record (config://assoc.list) lists every key, value and file the registration added and
// what it replaced; unregistering removes those (a value only while it still holds what was
// written) and puts back what was there.  Windows (win_assoc.rs): the setup's entries below
// HKCU\Software\Classes -- the ProgID <app>.<ext> (description, DefaultIcon, shell\open\command
// = "<exe>" "%1", AppUserModelID = the app id: what Windows asks of a handler with an explicit
// id for the recent files of the type), .<ext>\OpenWithProgids, for own types the default and
// Content Type of .<ext>, a scheme's key (URL:<display>, URL Protocol) -- then SHChangeNotify.
// A default of another program is the extension's default value (HKCU or HKLM) or a key of the
// same name with another command; the user's own choice in Windows' dialog (UserChoice, hashed) is never
// changed (Conflict::fixed).  Linux (linux_assoc.rs): $XDG_DATA_HOME/applications/<app_id>.desktop
// (Exec with %U for schemes, else %F; MimeType=), own types as shared-mime-info XML in
// $XDG_DATA_HOME/mime/packages/<app_id>.xml, the defaults of own types and schemes in
// $XDG_CONFIG_HOME/mimeapps.list [Default Applications] (written directly, no xdg-mime); the
// caches are refreshed by update-mime-database / update-desktop-database when they are there
// (without update-mime-database the own types are known after the next system update of the
// MIME database; the .desktop file needs no cache).  A default of another program is an entry
// in a mimeapps.list or, without one, another installed .desktop file with that MIME type.
// In an MSIX package or a Flatpak the package declares everything: Unavailable.  Web and
// Android: Unavailable.
//
// Not a permission of the manifest: only what PACKAGE declares is written, in the user's own
// part of the system (HKCU, $XDG_*_HOME).
//
// A web browser (PACKAGE browser=1, schemes= with http and https).  Windows: http and https get no
// keys of their own (HKCU\Software\Classes\http would stand in for the system's handler); the
// ProgID <app>URL opens web addresses (URL Protocol, the open command, Application with the
// name and icon), Software\Clients\StartMenuInternet\<app> is the browser of Default Programs
// (name, icon, the command, Capabilities: ApplicationName / Description / Icon, StartMenu,
// URLAssociations http / https -> <app>URL, FileAssociations .<ext> -> the file types' ProgIDs)
// and Software\RegisteredApplications\<app> names its Capabilities.  Windows 10 and later never
// let a program take the default itself: browser_request opens Settings at the app's page of the
// default apps (ms-settings:defaultapps?registeredAppUser=<app>), the user decides there;
// browser_status reads the user's choice of http and https (UserChoiceLatest, else UserChoice).
// Linux: the .desktop file says Network;WebBrowser; besides the MIME types; browser_status reads
// the defaults of x-scheme-handler/http and https from the mimeapps.list files (as xdg-mime and
// xdg-settings do), browser_request writes the app as the user's default of those and of
// text/html (and KDE's BrowserApplication in kdeglobals): the user asked for it with the call, and
// the desktops offer no question of their own.  macOS: browser_status asks Launch Services for
// http's handler, browser_request sets it for http and https -- macOS asks the user itself ("Do you
// want to change your default web browser?").  In a Flatpak, an MSIX package, the browser and on
// Android / iOS: Unknown and Unavailable.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

/// one file type of PACKAGE filetypes=
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileType {
    /// lower case, without the dot
    pub exts: Vec<String>,
    pub mime: String,
    pub desc: String,
    /// the app is the default program of these extensions
    pub own: bool,
}

/// what the app registers (apps/NAME/PACKAGE, built in by build_rs.py)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    /// the app's name (vfs::app_id): Windows ProgIDs <app>.<ext>
    pub app: String,
    /// PACKAGE display=: menus, "URL:<display>"
    pub display: String,
    /// the reverse-DNS id (PACKAGE app_id=, else org.jflib.<App>): the Linux .desktop id
    pub app_id: String,
    /// PACKAGE categories= (the .desktop file)
    pub categories: String,
    pub types: Vec<FileType>,
    pub schemes: Vec<String>,
    /// PACKAGE browser=1: the app is a web browser (registered as one; default_browser_status,
    /// default_browser_request)
    pub browser: bool,
}

/// a default of another program that a registration would replace
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// ".ext" (an own file type), "scheme:" (a URL scheme), a ProgID or a file that is in the way
    pub what: String,
    /// who has it now: a ProgID or command (Windows), a .desktop id or file (Linux)
    pub holder: String,
    /// the user chose that program in the system's dialog (Windows UserChoice): it stays,
    /// take is not asked
    pub fixed: bool,
}

/// what assoc_register did
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registered {
    /// the program the entries start
    pub exe: String,
    /// file types and schemes registered
    pub types: usize,
    pub schemes: usize,
    /// the defaults of other programs that stayed (take said no, or fixed)
    pub kept: Vec<Conflict>,
    /// hints (a missing cache tool ...)
    pub notes: Vec<String>,
}

/// why a registration failed
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AssocError {
    /// not here: web, Android, an MSIX package or a Flatpak (the package declares the types)
    Unavailable,
    /// PACKAGE has no filetypes= and no schemes=
    Empty,
    /// a file of another program is in the way and take said no (Linux: the .desktop file)
    Declined(Conflict),
    /// the system refused or failed: what it said
    Failed(String),
    /// (default_browser_request) the app is not registered as a web browser here: PACKAGE
    /// without browser=1, or neither the package nor --register registered this program
    NotRegistered,
}

impl core::fmt::Display for AssocError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            AssocError::Unavailable => f.write_str("file associations cannot be registered here (the package declares them)"),
            AssocError::Empty => f.write_str("PACKAGE declares no filetypes= and no schemes="),
            AssocError::Declined(c) => write!(f, "{} belongs to {} and was kept", c.what, c.holder),
            AssocError::Failed(s) => write!(f, "file associations: {s}"),
            AssocError::NotRegistered => f.write_str("the app is not registered as a web browser (PACKAGE browser=1, the package or --register)"),
        }
    }
}

static PACKAGE: SpinLock<Option<Package>> = SpinLock::new(None);

/// (ui::main!) keep the built-in PACKAGE text (JFGUI_PACKAGE)
#[doc(hidden)]
pub fn package_set(text: Option<&str>) {
    *PACKAGE.lock() = text.map(Package::parse);
}

/// what the app registers: the built-in PACKAGE, else the app's name without types
pub fn package() -> Package {
    if let Some(p) = PACKAGE.lock().clone() {
        return p;
    }
    Package::parse("")
}

impl Package {
    /// "key=value" lines: display, app_id, categories, filetypes, schemes, browser (the app is
    /// vfs::app_id; missing values get the defaults of tools/pkgmeta.py)
    pub fn parse(text: &str) -> Package {
        let app = crate::vfs::app_id();
        let mut p = Package { app: app.clone(), ..Package::default() };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "display" => p.display = String::from(v),
                "app_id" => p.app_id = String::from(v),
                "categories" => p.categories = String::from(v),
                "filetypes" => p.types = parse_types(v),
                "schemes" => p.schemes = parse_schemes(v),
                "browser" => p.browser = v == "1",
                _ => {}
            }
        }
        if p.display.is_empty() || p.display.contains(['\n', '\r', '\t']) {
            p.display = app.clone();
        }
        if p.app_id.is_empty() || !p.app_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.') {
            let clean: String = app.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            p.app_id = format!("org.jflib.{clean}");
        }
        if p.categories.is_empty() || p.categories.contains(['\n', '\r', '\t']) {
            p.categories = String::from("Utility;");
        }
        p
    }

    /// the MIME type of a file name by its extension (own file types first), None when unknown
    pub fn mime_of(&self, path: &str) -> Option<String> {
        let name = &path[path.rfind(['/', '\\']).map_or(0, |k| k + 1)..];
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        self.types.iter().find(|t| t.exts.contains(&ext)).map(|t| t.mime.clone())
    }
}

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_+.".contains(&c))
}

/// "ext[,ext..]|mime|description[|own]; ..." -> the valid entries
pub fn parse_types(text: &str) -> Vec<FileType> {
    let mut out = Vec::new();
    for entry in text.split(';') {
        let f: Vec<&str> = entry.split('|').map(str::trim).collect();
        if f.len() < 3 || f.len() > 4 || (f.len() == 4 && f[3] != "own") {
            continue;
        }
        let exts: Vec<String> = f[0].split(',').map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|e| name_ok(e)).collect();
        if exts.is_empty() || !f[1].contains('/') || f[1].contains(['\\', '"', '\t', '\n']) || f[2].contains(['"', '\t', '\n']) {
            continue;
        }
        out.push(FileType { exts, mime: String::from(f[1]), desc: String::from(f[2]), own: f.len() == 4 });
    }
    out
}

/// "scheme scheme; ..." -> the valid schemes (lower case)
pub fn parse_schemes(text: &str) -> Vec<String> {
    text.split([';', ' ', ',']).map(|s| s.trim().to_ascii_lowercase()).filter(|s| name_ok(s) && s.as_bytes()[0].is_ascii_alphabetic()).collect()
}

/// a value below HKCU (reg_entries): key, name ("" = the default value), REG_SZ value
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegValue {
    pub key: String,
    pub name: String,
    pub value: String,
    /// RV_TREE: the app's own key it lies in (a ProgID, a scheme), removed as a whole
    pub root: String,
    pub kind: u8,
}

/// a value in a key of the app's own (RegValue::root)
pub const RV_TREE: u8 = 0;
/// a value in a shared key (.ext\OpenWithProgids): only the value is the app's
pub const RV_SHARED: u8 = 1;
/// the default (and Content Type) of an extension: the app becomes its default program
pub const RV_DEFAULT: u8 = 2;

/// the HKCU values of the file types and schemes for the program exe (backslashes): the setup's
/// and assoc_register's.  app_id: the AppUserModelID the program runs under ("" = none, Windows
/// derives one): each ProgID names it (AppUserModelID), as Windows wants for the recent files
/// of an explicit id
pub fn reg_entries(app: &str, display: &str, exe: &str, types: &[FileType], schemes: &[String], app_id: &str) -> Vec<RegValue> {
    let mut out = Vec::new();
    let mut put = |key: &str, name: &str, value: &str, root: &str, kind: u8| {
        out.push(RegValue { key: String::from(key), name: String::from(name), value: String::from(value), root: String::from(root), kind });
    };
    let icon = format!("{exe},0");
    let cmd = format!("\"{exe}\" \"%1\"");
    let classes = "Software\\Classes";
    for t in types {
        let progid = format!("{app}.{}", t.exts[0]);
        let key = format!("{classes}\\{progid}");
        put(&key, "", &t.desc, &key, RV_TREE);
        put(&format!("{key}\\DefaultIcon"), "", &icon, &key, RV_TREE);
        put(&format!("{key}\\shell\\open\\command"), "", &cmd, &key, RV_TREE);
        if !app_id.is_empty() {
            put(&key, "AppUserModelID", app_id, &key, RV_TREE);
        }
        for e in &t.exts {
            let ext = format!("{classes}\\.{e}");
            put(&format!("{ext}\\OpenWithProgids"), &progid, "", "", RV_SHARED);
            if t.own {
                put(&ext, "", &progid, "", RV_DEFAULT);
                put(&ext, "Content Type", &t.mime, "", RV_DEFAULT);
            }
        }
    }
    for s in schemes {
        let key = format!("{classes}\\{s}");
        put(&key, "", &format!("URL:{display}"), &key, RV_TREE);
        put(&key, "URL Protocol", "", &key, RV_TREE);
        put(&format!("{key}\\DefaultIcon"), "", &icon, &key, RV_TREE);
        put(&format!("{key}\\shell\\open\\command"), "", &cmd, &key, RV_TREE);
    }
    out
}

/// the schemes a web browser opens through its ProgID <app>URL (Windows: no keys of their own)
const WEB_SCHEMES: [&str; 2] = ["http", "https"];

/// HKCU\Software\Clients\StartMenuInternet: the web browsers of Windows' Default Programs
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const BROWSER_CLIENTS: &str = "Software\\Clients\\StartMenuInternet";
/// HKCU\Software\RegisteredApplications: an app's name -> the key of its Capabilities
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const REGISTERED_APPS: &str = "Software\\RegisteredApplications";

/// a web browser's scheme (http, https): opened through <app>URL, no key of its own
pub(crate) fn web_scheme(pkg: &Package, s: &str) -> bool {
    pkg.browser && WEB_SCHEMES.contains(&s)
}

/// every HKCU value of pkg for the program exe: reg_entries of its file types and schemes and,
/// with browser=1, the web browser's registration in place of http's and https's own keys (the
/// ProgID <app>URL, Software\Clients\StartMenuInternet\<app> with its Capabilities, the value
/// <app> in Software\RegisteredApplications) -- what the setup and assoc_register write
pub fn package_entries(pkg: &Package, exe: &str) -> Vec<RegValue> {
    let schemes: Vec<String> = pkg.schemes.iter().filter(|s| !web_scheme(pkg, s)).cloned().collect();
    let mut out = reg_entries(&pkg.app, &pkg.display, exe, &pkg.types, &schemes, &pkg.app_id);
    if !pkg.browser {
        return out;
    }
    let mut put = |key: &str, name: &str, value: &str, root: &str, kind: u8| {
        out.push(RegValue { key: String::from(key), name: String::from(name), value: String::from(value), root: String::from(root), kind });
    };
    let (app, display) = (pkg.app.as_str(), pkg.display.as_str());
    let icon = format!("{exe},0");
    // the ProgID of web addresses
    let progid = format!("{app}URL");
    let url = format!("Software\\Classes\\{progid}");
    put(&url, "", &format!("{display} URL"), &url, RV_TREE);
    put(&url, "URL Protocol", "", &url, RV_TREE);
    put(&format!("{url}\\DefaultIcon"), "", &icon, &url, RV_TREE);
    put(&format!("{url}\\shell\\open\\command"), "", &format!("\"{exe}\" \"%1\""), &url, RV_TREE);
    let app_key = format!("{url}\\Application");
    put(&app_key, "ApplicationName", display, &url, RV_TREE);
    put(&app_key, "ApplicationIcon", &icon, &url, RV_TREE);
    if !pkg.app_id.is_empty() {
        put(&url, "AppUserModelID", &pkg.app_id, &url, RV_TREE);
        put(&app_key, "AppUserModelID", &pkg.app_id, &url, RV_TREE);
    }
    // the browser of Default Programs
    let smi = format!("{BROWSER_CLIENTS}\\{app}");
    let caps = format!("{smi}\\Capabilities");
    put(&smi, "", display, &smi, RV_TREE);
    put(&format!("{smi}\\DefaultIcon"), "", &icon, &smi, RV_TREE);
    put(&format!("{smi}\\shell\\open\\command"), "", &format!("\"{exe}\""), &smi, RV_TREE);
    put(&caps, "ApplicationName", display, &smi, RV_TREE);
    put(&caps, "ApplicationDescription", display, &smi, RV_TREE);
    put(&caps, "ApplicationIcon", &icon, &smi, RV_TREE);
    put(&format!("{caps}\\StartMenu"), "StartMenuInternet", app, &smi, RV_TREE);
    for s in pkg.schemes.iter().filter(|s| web_scheme(pkg, s)) {
        put(&format!("{caps}\\URLAssociations"), s, &progid, &smi, RV_TREE);
    }
    for t in &pkg.types {
        for e in &t.exts {
            put(&format!("{caps}\\FileAssociations"), &format!(".{e}"), &format!("{app}.{}", t.exts[0]), &smi, RV_TREE);
        }
    }
    put(REGISTERED_APPS, app, &caps, "", RV_SHARED);
    out
}

// ---------------------------------------------------------------- the web browser

/// (default_browser_status) is the app the system's web browser
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BrowserStatus {
    /// web addresses (http and https) open with this app
    Default,
    /// with another program (or the app is no browser: PACKAGE without browser=1)
    NotDefault,
    /// the system does not say: no default set (Linux), or not to be found out here (a
    /// Flatpak, the browser, Android, iOS)
    Unknown,
}

/// (PACKAGE browser=1) is this app the system's web browser (the built-in PACKAGE:
/// browser_status(&package())).  Ask again when the app's window is active again (UIC_ACTIVE):
/// the user may have changed it in the system's settings meanwhile
pub fn default_browser_status() -> BrowserStatus {
    browser_status(&package())
}

/// is the app of pkg the system's web browser: Windows -- the user's choice of http and https
/// (UserChoiceLatest, else UserChoice) is its ProgID <app>URL; Linux -- the mimeapps.list files'
/// defaults of x-scheme-handler/http and https name one of its .desktop files (none named:
/// Unknown); macOS -- Launch Services' handler of http and https is its bundle
pub fn browser_status(pkg: &Package) -> BrowserStatus {
    if !pkg.browser {
        return BrowserStatus::NotDefault;
    }
    native::browser_status(pkg)
}

/// (PACKAGE browser=1) let the user make this app the system's web browser (the built-in
/// PACKAGE: browser_request(&package())); call it from the user's click
pub fn default_browser_request() -> Result<(), AssocError> {
    browser_request(&package())
}

/// let the user make the app of pkg the web browser: Windows 10 and later -- Settings opens at
/// the app's page of the default apps (ms-settings:defaultapps?registeredAppUser=<app>; Windows
/// lets no program take the default itself), the user picks there; Linux -- the app's installed
/// .desktop file becomes the user's default of x-scheme-handler/http, https and text/html in
/// mimeapps.list (and KDE's BrowserApplication), as xdg-settings set default-web-browser does;
/// macOS -- Launch Services' handler of http and https, macOS asks the user.  Ok: asked or set
/// (read browser_status again later); NotRegistered: no browser=1, or this program is not
/// registered as a browser (install the package or run --register first); Unavailable: a
/// Flatpak, an MSIX package, the browser, Android, iOS.  jfgui_OPEN_DRYRUN=1 (Windows): the
/// page is printed, not opened
pub fn browser_request(pkg: &Package) -> Result<(), AssocError> {
    if !pkg.browser {
        return Err(AssocError::NotRegistered);
    }
    native::browser_request(pkg)
}

// ---------------------------------------------------------------- the record

const RECORD: &str = "config://assoc.list";
const RECORD_HEAD: &str = "jflib-assoc 1";

/// one record line: its tab-separated fields
pub(crate) type Line = Vec<String>;

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

fn record_text(exe: &str, lines: &[Line]) -> String {
    let mut s = String::from(RECORD_HEAD);
    s.push_str("\nexe\t");
    esc(exe, &mut s);
    s.push('\n');
    for l in lines {
        for (k, f) in l.iter().enumerate() {
            if k > 0 {
                s.push('\t');
            }
            esc(f, &mut s);
        }
        s.push('\n');
    }
    s
}

/// the record -> (exe, lines); None without one (or a damaged one)
fn record_read() -> Option<(String, Vec<Line>)> {
    let data = crate::read_file(RECORD)?;
    let text = String::from_utf8(data).ok()?;
    let mut it = text.lines();
    if it.next() != Some(RECORD_HEAD) {
        return None;
    }
    let mut exe = String::new();
    let mut lines = Vec::new();
    for l in it.filter(|l| !l.is_empty()) {
        let f: Line = l.split('\t').map(unesc).collect();
        if f[0] == "exe" {
            exe = f.get(1).cloned().unwrap_or_default();
        } else {
            lines.push(f);
        }
    }
    Some((exe, lines))
}

// ---------------------------------------------------------------- the API

/// the program the entries would start: this executable (Linux: the AppImage when it runs
/// from one)
#[cfg(any(windows, target_os = "linux"))]
fn program() -> Result<String, AssocError> {
    #[cfg(target_os = "linux")]
    if let Some(p) = crate::env("APPIMAGE").filter(|p| p.starts_with('/')) {
        return Ok(p);
    }
    let exe = crate::current_exe().ok_or_else(|| AssocError::Failed(String::from("no path of the program")))?;
    Ok(if cfg!(windows) { exe.replace('/', "\\") } else { exe })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn program() -> Result<String, AssocError> {
    Err(AssocError::Unavailable)
}

/// the defaults of other programs a registration of pkg would replace (nothing is written)
pub fn assoc_conflicts(pkg: &Package) -> Result<Vec<Conflict>, AssocError> {
    if pkg.types.is_empty() && pkg.schemes.is_empty() {
        return Err(AssocError::Empty);
    }
    native::available()?;
    let exe = program()?;
    let mine = record_read().map(|(_, l)| l).unwrap_or_default();
    Ok(native::conflicts(pkg, &exe, &mine))
}

/// register this program for pkg's file types and schemes; take(conflict) -> replace that
/// default of another program
pub fn assoc_register(pkg: &Package, take: &mut dyn FnMut(&Conflict) -> bool) -> Result<Registered, AssocError> {
    if pkg.types.is_empty() && pkg.schemes.is_empty() {
        return Err(AssocError::Empty);
    }
    native::available()?;
    let exe = program()?;
    assoc_unregister()?;
    let mut lines: Vec<Line> = Vec::new();
    let mut done = Registered { exe: exe.clone(), ..Registered::default() };
    if let Err(e) = native::register(pkg, &exe, take, &mut lines, &mut done) {
        native::undo(&lines);
        native::changed(&mut Vec::new());
        return Err(e);
    }
    let dir_ok = crate::vfs::app_dir(crate::vfs::APP_CONFIG).is_some_and(|d| crate::mkdir(&d));
    if !dir_ok || !crate::write_file(RECORD, record_text(&exe, &lines).as_bytes()) {
        native::undo(&lines);
        native::changed(&mut Vec::new());
        return Err(AssocError::Failed(String::from("the record config://assoc.list was not written")));
    }
    native::changed(&mut done.notes);
    Ok(done)
}

/// undo the registration of the record -> Ok(false) when there was none
pub fn assoc_unregister() -> Result<bool, AssocError> {
    let Some((_, lines)) = record_read() else {
        return if crate::file_exists(RECORD) == 1 {
            Err(AssocError::Failed(String::from("config://assoc.list is damaged")))
        } else {
            Ok(false)
        };
    };
    native::undo(&lines);
    if !crate::delete(RECORD) {
        return Err(AssocError::Failed(String::from("config://assoc.list was not removed")));
    }
    native::changed(&mut Vec::new());
    Ok(true)
}

/// the program a registration of this app starts (the record), None without one
pub fn assoc_registered() -> Option<String> {
    record_read().map(|(exe, _)| exe)
}

/// (ui::main!) Windows: PACKAGE's app id as the process's explicit AppUserModelID when this
/// program was installed by the setup (install.ini beside it names the id) or registered with
/// --register (the record names this program); jfgui_AUMID=ID gives one, =off none -> the id
/// set.  Other programs (and other systems) keep the id the system derives
#[doc(hidden)]
pub fn app_id_boot() -> Option<String> {
    #[cfg(windows)]
    {
        let id = match crate::env("jfgui_AUMID").as_deref() {
            Some("off") => return None,
            Some(id) if !id.is_empty() => String::from(id),
            _ => {
                let pkg = PACKAGE.lock().clone()?;
                let exe = program().ok()?;
                let line = format!("app_id={}", pkg.app_id);
                let installed = exe.rfind('\\')
                    .and_then(|k| crate::read_file_raw(&format!("{}\\install.ini", &exe[..k])))
                    .and_then(|d| String::from_utf8(d).ok())
                    .is_some_and(|t| t.lines().any(|l| l.trim() == line));
                let registered = assoc_registered().is_some_and(|p| p.replace('/', "\\").eq_ignore_ascii_case(&exe));
                if !installed && !registered {
                    return None;
                }
                pkg.app_id
            }
        };
        if crate::taskbar::set_app_user_model_id(&id) {
            return Some(id);
        }
    }
    None
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// %XX sequences decoded (None: a broken one or no UTF-8)
pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = hexval(*b.get(i + 1)?)?;
            let lo = hexval(*b.get(i + 2)?)?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// a path as a file:// URI: everything but unreserved characters and /:@!$&'()*+,;= escaped
/// (Windows: file:///C:/dir/a.txt)
pub(crate) fn file_uri(path: &str) -> String {
    let p = path.replace('\\', "/");
    let mut s = String::from(if p.starts_with('/') { "file://" } else { "file:///" });
    for &c in p.as_bytes() {
        if c.is_ascii_alphanumeric() || b"-._~/:@!$&'()*+,;=".contains(&c) {
            s.push(c as char);
        } else {
            s.push_str(&format!("%{c:02X}"));
        }
    }
    s
}

/// a local file:// URI as a path ("file:///home/a%20b.txt" -> "/home/a b.txt", Windows
/// "file:///C:/a.txt" -> "C:/a.txt"); None for anything else (another host, no URI)
pub fn file_uri_path(arg: &str) -> Option<String> {
    let rest = arg.strip_prefix("file://").or_else(|| arg.strip_prefix("FILE://"))?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let path = rest[slash..].split(['?', '#']).next().unwrap_or("");
    let path = percent_decode(path)?;
    if path.contains('\0') {
        return None;
    }
    let b = path.as_bytes();
    // /C:/dir -> C:/dir
    if cfg!(windows) && b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return Some(String::from(&path[1..]));
    }
    Some(path)
}

/// text on stdout, else (Windows, a GUI program started from a console) on the console of
/// the parent process
pub fn console_print(text: &str) {
    #[cfg(windows)]
    if native::console_print(text) {
        return;
    }
    crate::write_stdout(text.as_bytes());
}

/// (tests, Windows) HKCU\sub: name Some(n) its string value ("" = the default value), None:
/// Some("") when the key exists
#[cfg(windows)]
#[doc(hidden)]
pub fn hkcu_get(sub: &str, name: Option<&str>) -> Option<String> {
    native::hkcu_get(sub, name)
}

#[cfg(windows)]
#[path = "win_assoc.rs"]
mod native;

#[cfg(target_os = "linux")]
#[path = "linux_assoc.rs"]
mod native;

#[cfg(not(any(windows, target_os = "linux")))]
mod native {
    use super::{AssocError, Conflict, Line, Package, Registered};
    use alloc::string::String;
    use alloc::vec::Vec;

    pub fn available() -> Result<(), AssocError> {
        Err(AssocError::Unavailable)
    }
    pub fn conflicts(_: &Package, _: &str, _: &[Line]) -> Vec<Conflict> {
        Vec::new()
    }
    pub fn register(_: &Package, _: &str, _: &mut dyn FnMut(&Conflict) -> bool, _: &mut Vec<Line>, _: &mut Registered) -> Result<(), AssocError> {
        Err(AssocError::Unavailable)
    }
    pub fn undo(_: &[Line]) {}
    pub fn changed(_: &mut Vec<String>) {}
    #[cfg(target_os = "macos")]
    pub use super::mac_browser::{browser_request, browser_status};
    #[cfg(not(target_os = "macos"))]
    pub fn browser_status(_: &Package) -> super::BrowserStatus {
        super::BrowserStatus::Unknown
    }
    #[cfg(not(target_os = "macos"))]
    pub fn browser_request(_: &Package) -> Result<(), AssocError> {
        Err(AssocError::Unavailable)
    }
}

// macOS: the default web browser through Launch Services
#[cfg(target_os = "macos")]
#[path = "apple_assoc.rs"]
mod mac_browser;
