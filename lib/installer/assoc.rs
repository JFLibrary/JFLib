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
// Software\Classes\<scheme> ("URL:<display>", URL Protocol, DefaultIcon, shell\open\command).
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
}

/// the registry entries of the package's types, schemes and browser for the program exe
pub(crate) fn plan(pkg: &Package, exe: &str) -> Plan {
    let mut set = Vec::new();
    let mut remove: Vec<String> = Vec::new();
    for e in package_entries(pkg, exe) {
        let item = if e.kind == RV_TREE { format!("T:{}", e.root) } else { format!("V:{}|{}", e.key, e.name) };
        if !remove.contains(&item) {
            remove.push(item);
        }
        set.push((e.key, e.name, e.value));
    }
    Plan { set, remove: remove.join(";") }
}
