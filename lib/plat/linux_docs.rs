// linux_docs -- the documents portal (org.freedesktop.portal.Documents) over linux_dbus.rs
//
// A Flatpak app gets the files the user hands it (the portal's file chooser, a drop, "open
// with") as documents of the portal: below its FUSE mount, $XDG_RUNTIME_DIR/doc/ID/NAME
// (/run/user/UID/doc/...), ordinary OS paths.  The file chooser exports lasting documents (the
// ID stays with the app's permissions across log-outs and reboots); flatpak's file forwarding
// ("open with") and the file transfer portal make documents for the session only.
// persistent() asks the portal for the lasting document of a path below the mount:
// Documents.Add(O_PATH descriptor, reuse_existing, persistent) -> an ID, the same one when the
// same file is asked again.  For a descriptor on its own mount the portal answers that
// document's ID (xdg-document-portal 1.18, Documents version 4: it refuses reuse_existing
// false), so a session document cannot be made lasting from inside the sandbox; where the
// portal reaches the real file the answer is a lasting document of it.
//
//   persistent(path) -> Option<String>   <mount>/<ID of the answer>/<NAME> for a file document
//        below the mount; None for any other path, a folder or a file inside an exported
//        folder, no session bus / portal / descriptor passing, or the portal's refusal
//   sandboxed(path) -> bool   inside a Flatpak sandbox, below the mount (policy.rs: such
//        documents are the user's, allowed with fs.user like Android's content://, so a lasting
//        one opens again after a restart without a grant)
//
// The mount is $XDG_RUNTIME_DIR/doc (else /run/user/UID/doc); persistent() checks it against
// the portal's GetMountPoint once per process.  Tested against a fake portal on a private bus
// (tools/platshared/docs_bus.py, plattest --test docs-bus) and with --real against
// xdg-document-portal and xdg-desktop-portal's file chooser for an app in a bwrap sandbox.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use rt::os::syscall6;
use rt::sync::SpinLock;

use crate::linux_dbus::{Conn, Msg, Value};

const SYS_CLOSE: usize = 3;
const SYS_GETUID: usize = 102;
const SYS_OPENAT: usize = 257;
const AT_FDCWD: isize = -100;
const O_PATH: usize = 0x20_0000;
const O_CLOEXEC: usize = 0x8_0000;
const T_PORTAL: i32 = 5_000; // ms until the portal answers (it may be started first)
const NAME: &str = "org.freedesktop.portal.Documents";
const PATH: &str = "/org/freedesktop/portal/documents";

// the portal's mount point, asked once (Some(None): no portal)
static MOUNT: SpinLock<Option<Option<String>>> = SpinLock::new(None);

// where the portal mounts its documents: $XDG_RUNTIME_DIR/doc, else /run/user/UID/doc
fn base() -> String {
    match crate::env("XDG_RUNTIME_DIR").filter(|d| d.starts_with('/')) {
        Some(d) => format!("{}/doc", d.trim_end_matches('/')),
        // SAFETY: getuid has no arguments.
        None => format!("/run/user/{}/doc", unsafe { syscall6(SYS_GETUID, 0, 0, 0, 0, 0, 0) }),
    }
}

// (ID, NAME) of a file document's path below the mount
fn split(path: &str) -> Option<(&str, &str)> {
    let b = base();
    let rest = path.strip_prefix(b.as_str())?.strip_prefix('/')?;
    let (id, name) = rest.split_once('/')?;
    let id_ok = !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric());
    (id_ok && !name.is_empty() && !name.contains('/')).then_some((id, name))
}

/// see the file header
pub(crate) fn sandboxed(path: &str) -> bool {
    path.strip_prefix(base().as_str()).is_some_and(|r| r.starts_with('/')) && crate::linux_desk::in_flatpak()
}

// the portal's mount point (GetMountPoint -> ay), asked once (until it answers)
fn mount(c: &mut Conn) -> Option<String> {
    let mut g = MOUNT.lock();
    if let Some(m) = g.as_ref() {
        return m.clone();
    }
    // no answer at all (the portal is still starting): not kept, asked again next time
    let r = crate::linux_desk::call(c, &Msg::call(NAME, PATH, NAME, "GetMountPoint", Vec::new()), T_PORTAL)?;
    let m = Some(r).and_then(|r| match r.body.first() {
        Some(Value::A(_, bytes)) => {
            let b: Vec<u8> = bytes.iter().filter_map(|v| if let Value::Y(c) = v { Some(*c) } else { None }).take_while(|&c| c != 0).collect();
            String::from_utf8(b).ok().filter(|s| s.starts_with('/'))
        }
        _ => None,
    });
    *g = Some(m.clone());
    m
}

/// see the file header
pub(crate) fn persistent(path: &str) -> Option<String> {
    let (_, name) = split(path)?;
    let mut c = Conn::session_fds()?;
    if !c.fds_ok || mount(&mut c)?.trim_end_matches('/') != base() {
        return None;
    }
    let mut z = Vec::from(path.as_bytes());
    z.push(0);
    // SAFETY: openat with a NUL-terminated path; the descriptor is closed below.
    let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD as usize, z.as_ptr() as usize, O_PATH | O_CLOEXEC, 0, 0, 0) };
    if fd < 0 {
        return None;
    }
    // Add(h o_path_fd, b reuse_existing, b persistent) -> s doc_id
    let mut m = Msg::call(NAME, PATH, NAME, "Add", vec![Value::H(0), Value::B(true), Value::B(true)]);
    m.fds = vec![fd as i32];
    let r = crate::linux_desk::call(&mut c, &m, T_PORTAL);
    // SAFETY: our descriptor, closed once (the bus copied it).
    unsafe { syscall6(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0) };
    let id = String::from(r?.body.first()?.as_str()?);
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(format!("{}/{id}/{name}", base()))
}
