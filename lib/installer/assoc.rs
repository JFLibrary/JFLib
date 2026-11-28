// assoc -- the app's file types and URL schemes (apps/APP/PACKAGE filetypes= / schemes=, carried
// in the payload manifest): the HKCU\Software\Classes entries the setup writes, and the list
// install.ini keeps for the uninstaller
//
//   parse_types(text) -> [FileType]   "ext[,ext..]|mime|description[|own]; ..." -- every
//                                     extension gets the app in "Open with"; "own" types also
//                                     open with it by default (their extension is the app's)
//   parse_schemes(text) -> [scheme]   "scheme scheme ..." (';' or blanks)
//   plan(pkg, exe) -> Plan { set: [(subkey, name, value)], remove }
//                                     remove: "T:<subkey>" (the key with everything below) and
//                                     "V:<subkey>|<name>" (one value), ';' separated
//
// The entries are plat::assoc::package_entries -- the same ones a developer build writes with
// --register (lib/plat/assoc.rs): Software\Classes\<app>.<ext> (the ProgID: description,
// DefaultIcon, shell\open\command = "<exe>" "%1", AppUserModelID = the app id),
// Software\Classes\.<ext>\OpenWithProgids\<ProgID>,
// for "own" types also the default value and Content Type of Software\Classes\.<ext>; a scheme:
// Software\Classes\<scheme> ("URL:<display>", URL Protocol, DefaultIcon, shell\open\command =
// "<exe>" -- "%1": a quote in the URL cannot add options).
// A web browser (manifest browser=1): http and https through the ProgID <app>URL,
// Software\Clients\StartMenuInternet\<app> (Default Programs: Capabilities with the URL and
// file associations) and the value <app> in Software\RegisteredApplications; the user makes it
// the default in Settings (the app: plat::default_browser_request).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub(crate) use plat::assoc::{Package, parse_schemes, parse_types};
use plat::assoc::{package_entries, RV_TREE};

/// what the setup writes and what the uninstaller removes
pub(crate) struct Plan {
    pub(crate) set: Vec<(String, String, String)>,
    pub(crate) remove: String,
    /// keys of the app's own that another program holds (a scheme such as mailto): left alone
    pub(crate) kept: Vec<String>,
}

/// whether a key's command (`have`) starts the same program as `want`: the same command, or the
/// same program file name in its first quoted part (an earlier setup of the app in another folder)
pub(crate) fn same_program(have: &str, want: &str) -> bool {
    let prog = |c: &str| -> Option<String> {
        let p = c.trim_start().strip_prefix('"')?.split('"').next()?;
        Some(p.rsplit(['\\', '/']).next().unwrap_or(p).to_ascii_lowercase())
    };
    have.trim().eq_ignore_ascii_case(want.trim()) || prog(have).is_some_and(|h| prog(want) == Some(h))
}

/// the registry entries of the package's types, schemes and browser for the program exe.
/// `holder(key)`: the command of a key that exists already (None: missing).  A key of the app's
/// own that exists with another program's command is not written and not removed: a setup for
/// all users with schemes=mailto overwrote the computer's mail handler, and its uninstaller
/// removed the key, for every user
pub(crate) fn plan(pkg: &Package, exe: &str, holder: &dyn Fn(&str) -> Option<String>) -> Plan {
    let entries = package_entries(pkg, exe);
    let mut kept: Vec<String> = Vec::new();
    let mut looked: Vec<&str> = Vec::new();
    for e in entries.iter().filter(|e| e.kind == RV_TREE) {
        if looked.contains(&e.root.as_str()) {
            continue;
        }
        looked.push(&e.root);
        let key = format!("{}\\shell\\open\\command", e.root);
        let mine = entries.iter().find(|x| x.key.eq_ignore_ascii_case(&key) && x.name.is_empty()).map_or("", |x| x.value.as_str());
        if holder(&e.root).is_some_and(|c| !same_program(&c, mine)) {
            kept.push(e.root.clone());
        }
    }
    let mut set = Vec::new();
    let mut remove: Vec<String> = Vec::new();
    for e in entries.iter().filter(|e| !(e.kind == RV_TREE && kept.contains(&e.root))) {
        let item = if e.kind == RV_TREE { format!("T:{}", e.root) } else { format!("V:{}|{}", e.key, e.name) };
        if !remove.contains(&item) {
            remove.push(item);
        }
        set.push((e.key.clone(), e.name.clone(), e.value.clone()));
    }
    Plan { set, remove: remove.join(";"), kept }
}
