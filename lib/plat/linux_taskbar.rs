// linux_taskbar -- (taskbar.rs) the dock's progress and counter on Linux: the Unity launcher API
//
//   progress(&State) / badge(&State)   the signal com.canonical.Unity.LauncherEntry.Update
//                     (s app_uri, a{sv} properties) from /com/canonical/unity/launcherentry/<n>
//                     with progress (d), progress-visible (b), count (x), count-visible (b);
//                     app_uri = application://<desktop id>.desktop (linux_desk.rs)
//   tasks(..)         false: Linux docks take quick lists from the .desktop file's Actions
//   supported()       a session bus is announced
//   set_app_id(..)    false: the desktop knows the app by its .desktop id (linux_desk.rs)
//
// The signals go out on a session-bus connection kept open (linux_desk.rs Link): docks forget
// an entry's state when its sender goes away.  A dock that starts later may ask
// LauncherEntry.Query() -> (app_uri, properties); it is answered whenever the connection is
// used, and the full state goes out with every update.

use alloc::string::String;

use super::{JumpTask, State, TaskbarProgress};
use crate::linux_dbus::{Msg, Value, session_available};
use crate::linux_desk::{Link, desktop_id, drain};

const IFACE: &str = "com.canonical.Unity.LauncherEntry";

static LINK: Link = Link::new();

fn app_uri() -> String {
    alloc::format!("application://{}.desktop", desktop_id())
}

// the object path: one per app uri (FNV-1a of it, as libunity hashes the id)
fn path(uri: &str) -> String {
    let h = uri.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    alloc::format!("/com/canonical/unity/launcherentry/{h}")
}

fn props(s: &State) -> Value {
    let bar = !matches!(s.progress, TaskbarProgress::None | TaskbarProgress::Indeterminate);
    Value::dict(alloc::vec![
        ("progress", Value::D(s.progress.value() as f64)),
        ("progress-visible", Value::B(bar)),
        ("count", Value::X(s.count as i64)),
        ("count-visible", Value::B(s.count != 0)),
    ])
}

fn update(s: &State) -> bool {
    let uri = app_uri();
    let at = path(&uri);
    LINK.with(|c| {
        drain(c, |m| {
            (m.iface == IFACE && m.member == "Query" && m.path == at).then(|| alloc::vec![Value::S(uri.clone()), props(&super::state())])
        });
        c.send(&Msg::signal(&at, IFACE, "Update", alloc::vec![Value::S(uri.clone()), props(s)])).is_some()
    })
    .unwrap_or(false)
}

pub fn progress(s: &State) -> bool {
    update(s)
}

pub fn badge(s: &State) -> bool {
    update(s)
}

pub fn tasks(_tasks: &[JumpTask]) -> bool {
    false
}

pub fn supported() -> bool {
    session_available()
}

pub fn set_app_id(_id: &str) -> bool {
    false
}

pub fn app_id() -> Option<String> {
    None
}
