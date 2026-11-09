// apple_autostart -- (autostart.rs) macOS: a LaunchAgent of the user starts the app at login
//
//   set(on, args)   ~/Library/LaunchAgents/<app id>.plist: Label, ProgramArguments (the running
//                   executable and args), RunAtLoad, LimitLoadToSessionType Aqua (only in the logged-in
//                   session), ProcessType Interactive; written whole, replacing an older one.  Off:
//                   the file goes.  macOS tells the user about a new login item itself.
//   get()           the file is there and not Disabled
//   entry()         its ProgramArguments, quoted where needed ("exe" args)
//
// launchd reads the folder at the next login; nothing is loaded now (the app runs already).  A bundled
// app (AP7c) could use SMAppService instead; the LaunchAgent works for any program.  $HOME decides
// where the folder is (tests point it elsewhere).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::autostart::AutostartError;

fn file() -> Option<String> {
    let home = crate::env("HOME").filter(|h| h.starts_with('/'))?;
    let id = crate::vfs::app_id();
    if id.is_empty() || id.contains(['/', '\0']) {
        return None;
    }
    Some(format!("{}/Library/LaunchAgents/{id}.plist", home.trim_end_matches('/')))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unesc(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// the property list of the agent
pub(crate) fn plist(label: &str, exe: &str, args: &[&str]) -> String {
    let mut a = format!("\t\t<string>{}</string>\n", esc(exe));
    for x in args {
        a.push_str(&format!("\t\t<string>{}</string>\n", esc(x)));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>{}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n{a}\t</array>\n\t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>LimitLoadToSessionType</key>\n\t<string>Aqua</string>\n\t<key>ProcessType</key>\n\t<string>Interactive</string>\n</dict>\n</plist>\n",
        esc(label)
    )
}

pub fn set(on: bool, args: &[&str]) -> Result<(), AutostartError> {
    let f = file().ok_or(AutostartError::Unavailable)?;
    if !on {
        if crate::file_exists_raw(&f) != 0 && !crate::delete_raw(&f) {
            return Err(AutostartError::Failed(format!("{f} could not be removed")));
        }
        return Ok(());
    }
    let exe = crate::current_exe().ok_or(AutostartError::Unavailable)?;
    let text = plist(&crate::vfs::app_id(), &exe, args);
    if text.len() > 256 * 1024 {
        return Err(AutostartError::Invalid);
    }
    let dir = &f[..f.rfind('/').unwrap_or(0)];
    if crate::file_exists_raw(dir) != 2 && !crate::mkdir_raw(dir) {
        return Err(AutostartError::Failed(format!("{dir} could not be made")));
    }
    if !crate::write_file_raw(&f, text.as_bytes()) {
        return Err(AutostartError::Failed(format!("{f} could not be written")));
    }
    Ok(())
}

/// the agent's text, when there is one
fn read() -> Option<String> {
    let f = file()?;
    let b = crate::read_file_raw(&f)?;
    String::from_utf8(b).ok()
}

pub fn get() -> bool {
    read().is_some_and(|t| !t.replace(['\t', '\n', ' '], "").contains("<key>Disabled</key><true/>"))
}

/// the strings of ProgramArguments
fn program_args(t: &str) -> Vec<String> {
    let Some(k) = t.find("<key>ProgramArguments</key>") else { return Vec::new() };
    let rest = &t[k..];
    let end = rest.find("</array>").unwrap_or(rest.len());
    let mut out = Vec::new();
    let mut s = &rest[..end];
    while let Some(a) = s.find("<string>") {
        let Some(b) = s[a..].find("</string>") else { break };
        out.push(unesc(&s[a + 8..a + b]));
        s = &s[a + b + 9..];
    }
    out
}

pub fn entry() -> Option<String> {
    let args = program_args(&read()?);
    if args.is_empty() {
        return None;
    }
    let words: Vec<String> = args
        .iter()
        .map(|a| if a.is_empty() || a.contains([' ', '"', '\\', '\'']) { format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")) } else { a.clone() })
        .collect();
    Some(words.join(" "))
}
