// assoc -- the app's file types and URL schemes at start-up (plat::assoc): the switches every
// app gets through ui::main!
//
//   --register            register this program for PACKAGE filetypes= / schemes= (Windows:
//                         HKCU\Software\Classes; Linux: a .desktop file, the own MIME types,
//                         mimeapps.list); defaults of other programs stay and are listed
//   --register-replace    the same, and those defaults go to this app too (the user's own pick
//                         in Windows' dialog always stays)
//   --unregister          remove exactly what the registration added, put back what it replaced
// They print what they did (a Windows GUI program: on the console it was started from) and end
// the program: exit code 0 done, 1 failed, 2 not possible here or nothing declared.
//
//   desk_boot(package) -> Option<exit code>   (main!) keep the built-in PACKAGE text
//                         (plat::assoc::package_set), run a switch; headless runs (--shot,
//                         --script, the agent protocols) leave the system's list of recent files
//                         alone (plat::recent::recent_os(false); jfgui_RECENT still redirects it).
//                         Without a switch, Windows: the app id as the explicit AppUserModelID
//                         once the setup installed the program or --register registered it
//                         (plat::assoc::app_id_boot), before the first window
//   assoc_switch(pkg, switch) -> Option<(exit code, text)>   one switch for pkg (None: no switch)

use alloc::format;
use alloc::string::String;

use plat::assoc::{self, AssocError, Conflict, Package};

fn error_code(e: &AssocError) -> i32 {
    match e {
        AssocError::Unavailable | AssocError::Empty => 2,
        _ => 1,
    }
}

/// (main!) PACKAGE, then the switches -> Some(exit code) when one of them ran
#[doc(hidden)]
pub fn desk_boot(package: Option<&str>) -> Option<i32> {
    assoc::package_set(package);
    let args = plat::args();
    let has = |o: &str| args.iter().skip(1).any(|a| a == o);
    // as perm.rs: the runs without a window
    let headless = ["--shot", "--script"].iter().any(|o| plat::arg_value(o).is_some()) || has("--agent-json") || has("--agent-mcp");
    if headless {
        plat::recent::recent_os(false);
    }
    let Some(switch) = ["--unregister", "--register-replace", "--register"].into_iter().find(|s| has(s)) else {
        assoc::app_id_boot();
        return None;
    };
    let (code, text) = assoc_switch(&assoc::package(), switch)?;
    assoc::console_print(&text);
    Some(code)
}

/// run --register / --register-replace / --unregister for pkg -> (exit code, what it says)
pub fn assoc_switch(pkg: &Package, switch: &str) -> Option<(i32, String)> {
    if switch == "--unregister" {
        return Some(match assoc::assoc_unregister() {
            Ok(true) => (0, String::from("unregistered: the file types and URL schemes are removed\n")),
            Ok(false) => (0, String::from("nothing to unregister\n")),
            Err(e) => (error_code(&e), format!("error: {e}\n")),
        });
    }
    let replace = match switch {
        "--register" => false,
        "--register-replace" => true,
        _ => return None,
    };
    let mut take = |_: &Conflict| replace;
    Some(match assoc::assoc_register(pkg, &mut take) {
        Ok(r) => {
            let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
            let mut out = format!("registered {}: {}, {}\n", r.exe, plural(r.types, "file type", "file types"), plural(r.schemes, "URL scheme", "URL schemes"));
            for c in &r.kept {
                let why = if c.fixed { "the user's choice in Windows' settings" } else { "--register-replace takes it" };
                out.push_str(&format!("kept: {} stays with {} ({why})\n", c.what, c.holder));
            }
            for n in &r.notes {
                out.push_str(&format!("note: {n}\n"));
            }
            (0, out)
        }
        Err(e) => {
            let mut out = format!("error: {e}\n");
            if let AssocError::Declined(_) = e {
                out.push_str("(--register-replace moves it aside; --unregister puts it back)\n");
            }
            (error_code(&e), out)
        }
    })
}
