// linux_autostart -- (autostart.rs) XDG autostart on Linux, the Background portal in Flatpak
//
//   set(on, args)   file: $XDG_CONFIG_HOME/autostart/<desktop id>.desktop written (Type,
//                   Name, Exec, Icon, X-GNOME-Autostart-enabled=true) or removed.  Portal:
//                   org.freedesktop.portal.Background.RequestBackground("", {handle_token,
//                   reason, autostart, commandline (only with arguments), dbus-activatable
//                   false}) and its Request's Response (0 = granted; results "autostart"
//                   must match); the answer is remembered in config://autostart
//   get() / entry() the file (Hidden=true, X-GNOME-Autostart-enabled=false: off; its Exec) or
//                   what config://autostart remembers
//   (crate) exec_line(exe, args)   an Exec value: arguments with reserved characters in double
//                   quotes (\" \` \$ \\ inside), % as %%, then the string escapes \\ \n \t \r
//
// The way: the portal inside Flatpak (/.flatpak-info), else the file; jfgui_AUTOSTART=portal
// or =file forces one (tests).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::{AUTOSTART_WAIT_MS, AutostartError};
use crate::linux_dbus::{Conn, Msg, T_ERROR, T_RETURN, T_SIGNAL, Value};
use crate::linux_desk::{desktop_id, in_flatpak};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const MEMO: &str = "config://autostart";
static TOKEN: AtomicU32 = AtomicU32::new(0);

fn use_portal() -> bool {
    match crate::env("jfgui_AUTOSTART").as_deref() {
        Some("portal") => true,
        Some("file") => false,
        _ => in_flatpak(),
    }
}

/// one Exec argument (Desktop Entry spec: quoting, field codes) into out, before the string
/// escapes
fn exec_arg(a: &str, out: &mut String) {
    const RESERVED: &str = " \t\n\r\"'\\><~|&;$*?#()`";
    if !a.is_empty() && !a.contains(|c| RESERVED.contains(c)) {
        out.push_str(a);
        return;
    }
    out.push('"');
    for c in a.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

/// the Exec value that starts exe with args
pub(crate) fn exec_line(exe: &str, args: &[&str]) -> String {
    let mut quoted = String::new();
    for (k, a) in core::iter::once(&exe).chain(args.iter()).enumerate() {
        if k > 0 {
            quoted.push(' ');
        }
        exec_arg(a, &mut quoted);
    }
    let mut out = String::new();
    for c in quoted.chars() {
        match c {
            '%' => out.push_str("%%"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

// the program to start: the AppImage when this runs from one
fn program() -> Result<String, AutostartError> {
    if let Some(p) = crate::env("APPIMAGE").filter(|p| p.starts_with('/')) {
        return Ok(p);
    }
    crate::current_exe().ok_or_else(|| AutostartError::Failed(String::from("no path of the program")))
}

fn file_path() -> Option<String> {
    Some(format!("{}/autostart/{}.desktop", crate::config_dir()?, desktop_id()))
}

pub fn set(on: bool, args: &[&str]) -> Result<(), AutostartError> {
    if use_portal() {
        return portal_set(on, args);
    }
    let path = file_path().ok_or(AutostartError::Unavailable)?;
    if !on {
        if crate::file_exists_raw(&path) == 0 || crate::delete_raw(&path) {
            return Ok(());
        }
        return Err(AutostartError::Failed(format!("{path} was not removed")));
    }
    let exec = exec_line(&program()?, args);
    let id = desktop_id();
    let app = crate::vfs::app_id();
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={app}\nExec={exec}\nIcon={id}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    let dir = &path[..path.rfind('/').unwrap_or(0)];
    if !crate::mkdir_raw(dir) || !crate::write_file_raw(&path, text.as_bytes()) {
        return Err(AutostartError::Failed(format!("{path} was not written")));
    }
    Ok(())
}

// the [Desktop Entry] keys of the file -> (switched on, Exec)
fn read_file() -> Option<(bool, String)> {
    let data = crate::read_file_raw(&file_path()?)?;
    let text = String::from_utf8_lossy(&data);
    let (mut on, mut exec, mut group) = (true, String::new(), false);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            group = line == "[Desktop Entry]";
            continue;
        }
        let Some((k, v)) = line.split_once('=').filter(|_| group) else { continue };
        match (k.trim(), v.trim()) {
            ("Hidden", "true") | ("X-GNOME-Autostart-enabled", "false") => on = false,
            ("Exec", v) => exec = String::from(v),
            _ => {}
        }
    }
    Some((on, exec))
}

pub fn get() -> bool {
    if use_portal() {
        return crate::file_exists(MEMO) == 1;
    }
    read_file().is_some_and(|(on, _)| on)
}

pub fn entry() -> Option<String> {
    if use_portal() {
        return crate::read_file(MEMO).map(|d| String::from_utf8_lossy(&d).into_owned());
    }
    read_file().map(|(_, exec)| exec)
}

// ---------------------------------------------------------------- the Background portal

fn portal_set(on: bool, args: &[&str]) -> Result<(), AutostartError> {
    let mut c = Conn::session().ok_or(AutostartError::Unavailable)?;
    let token = format!("jfgui_bg{}", TOKEN.fetch_add(1, Ordering::Relaxed) + 1);
    let sender = c.unique.trim_start_matches(':').replace('.', "_");
    let mut handle = format!("{PORTAL_PATH}/request/{sender}/{token}");
    let rule = format!("type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{handle}'");
    c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", alloc::vec![Value::S(rule)]));
    let exe = program()?;
    let mut opts = alloc::vec![
        ("handle_token", Value::S(token)),
        ("reason", Value::str("Beim Anmelden starten")),
        ("autostart", Value::B(on)),
        ("dbus-activatable", Value::B(false)),
    ];
    if on && !args.is_empty() {
        let cmd: Vec<Value> = core::iter::once(exe.as_str()).chain(args.iter().copied()).map(Value::str).collect();
        opts.push(("commandline", Value::A(String::from("s"), cmd)));
    }
    let call = Msg::call(PORTAL, PORTAL_PATH, "org.freedesktop.portal.Background", "RequestBackground", alloc::vec![Value::str(""), Value::dict(opts)]);
    let serial = c.send(&call).ok_or(AutostartError::Unavailable)?;
    let end = crate::time_us() + AUTOSTART_WAIT_MS as u64 * 1000;
    loop {
        let now = crate::time_us();
        if now >= end {
            return Err(AutostartError::Dismissed);
        }
        let Some(m) = c.read(((end - now) / 1000) as i32 + 1) else {
            if c.broken {
                return Err(AutostartError::Failed(String::from("the session bus closed the connection")));
            }
            continue;
        };
        match m.typ {
            T_ERROR if m.reply_to == serial => {
                let missing = m.error.ends_with(".ServiceUnknown") || m.error.ends_with(".UnknownMethod") || m.error.ends_with(".UnknownInterface");
                return Err(if missing { AutostartError::Unavailable } else { AutostartError::Failed(m.error) });
            }
            T_RETURN if m.reply_to == serial => {
                if let Some(p) = m.body.first().and_then(|v| v.as_str()) {
                    handle = String::from(p); // older portals choose the path themselves
                }
            }
            T_SIGNAL if m.member == "Response" && m.path == handle => {
                let code = m.body.first().and_then(|v| v.as_u32()).unwrap_or(2);
                let granted = match m.body.get(1) {
                    Some(Value::A(_, kv)) => kv.iter().find_map(|e| match e {
                        Value::E(k, v) if k.as_str() == Some("autostart") => v.as_u32(),
                        _ => None,
                    }),
                    _ => None,
                };
                if code != 0 || granted.is_some_and(|g| (g != 0) != on) {
                    return Err(AutostartError::Dismissed);
                }
                if on {
                    crate::write_file(MEMO, exec_line(&exe, args).as_bytes());
                } else {
                    crate::delete(MEMO);
                }
                return Ok(());
            }
            _ => {}
        }
    }
}
