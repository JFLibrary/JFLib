// linux_tray -- (tray.rs) the symbol in the notification area and the notifications on Linux,
// on a session-bus connection of a thread of its own (linux_dbus.rs)
//
//   tray_set / tray_remove / notify / notify_close / supported   (tray.rs has the contract)
//
// The symbol is a StatusNotifierItem: the name org.kde.StatusNotifierItem-<pid>-1 (when it is
// taken -- in Flatpak the pid is the same for every app -- the connection's own name and the
// object path), the object /StatusNotifierItem (properties Category, Id, Title, Status,
// IconName / IconPixmap, ToolTip, ItemIsMenu, Menu; Activate -> Activate, SecondaryActivate ->
// SecondaryActivate; KDE's ProvideXdgActivationToken before them -> the token for the window
// the app raises, linux_wl_act.rs), announced to org.kde.StatusNotifierWatcher (again when the watcher
// starts anew).  supported() is whether that watcher runs (KDE, Xfce, ...; GNOME only with the
// AppIndicator extension): asked on the bus, kept up to date by the thread.  Its menu is a
// com.canonical.dbusmenu at /MenuBar (GetLayout, GetGroupProperties, Event "clicked" ->
// Menu(id), AboutToShow); every tray_set raises the layout revision.  tray_remove gives the
// name back.  Notifications: org.freedesktop.Notifications.Notify with the action "default";
// ActionInvoked -> NoteClicked (the ActivationToken signal before it -> the token, as for the
// symbol), NotificationClosed -> NoteClosed (unless it was clicked).
// The UI thread hands its requests over in a queue and wakes the thread with an eventfd.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use rt::os::syscall6;
use rt::sync::SpinLock;

use crate::linux_dbus::{Conn, Msg, T_CALL, T_ERROR, T_RETURN, T_SIGNAL, Value, session_available};
use crate::tray::{self, Note, TI_CHECKED, TI_DISABLED, TI_SEPARATOR, Tray, TrayEvent, TrayIcon, TrayItem};

const SNI_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const SNI_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const NOTES: &str = "org.freedesktop.Notifications";

enum Op {
    Set(Tray),
    Remove,
    Notify(u32, Note),
    Close(u32),
}

static OPS: SpinLock<Vec<Op>> = SpinLock::new(Vec::new());
static WAKE: AtomicI32 = AtomicI32::new(-1);
// 0 not started, 1 running, 2 no bus
static STATE: AtomicU32 = AtomicU32::new(0);
// the StatusNotifierWatcher runs: 1 yes, 0 no, -1 not known yet (the thread asks the bus at
// its start and follows NameOwnerChanged)
static WATCHER_UP: AtomicI32 = AtomicI32::new(-1);

// the thread runs (started now if not) -> false without a session bus
fn ensure() -> bool {
    if STATE.load(Ordering::Acquire) == 2 || !session_available() {
        return false;
    }
    if STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
        // SAFETY: eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK).
        let fd = unsafe { syscall6(290, 0, 0x80000 | 0x800, 0, 0, 0, 0) };
        WAKE.store(fd as i32, Ordering::Release);
        if crate::thread_spawn(256 << 10, Box::new(run)).is_none() {
            STATE.store(2, Ordering::Release);
            return false;
        }
    }
    true
}

fn queue(op: Op) -> bool {
    if !ensure() {
        return false;
    }
    OPS.lock().push(op);
    let fd = WAKE.load(Ordering::Acquire);
    if fd >= 0 {
        let one = 1u64;
        // SAFETY: an 8-byte counter increment on our eventfd.
        unsafe { syscall6(1, fd as usize, &one as *const u64 as usize, 8, 0, 0, 0) };
    }
    true
}

/// queued in any case (shown as soon as a watcher appears); true when it is shown now
pub fn tray_set(t: &Tray) -> bool {
    queue(Op::Set(t.clone())) && supported()
}

pub fn tray_remove() {
    queue(Op::Remove);
}

pub fn notify(id: u32, n: &Note) -> bool {
    queue(Op::Notify(id, n.clone()))
}

pub fn notify_close(id: u32) {
    queue(Op::Close(id));
}

/// a notification area shows the symbol: the StatusNotifierWatcher runs (the first call waits
/// up to 1 s for the thread's answer from the bus; the thread follows its comings and goings)
pub fn supported() -> bool {
    if !ensure() {
        return false;
    }
    for _ in 0..200 {
        let up = WATCHER_UP.load(Ordering::Acquire);
        if up >= 0 {
            return up == 1;
        }
        if STATE.load(Ordering::Acquire) == 2 {
            return false;
        }
        crate::sleep_ms(5);
    }
    false
}

// ---------------------------------------------------------------- the thread

struct Item {
    tray: Option<Tray>,
    /// the item is announced (its signals go out)
    named: bool,
    /// the name service() is ours (else the item is known by the connection's name and path)
    own_name: bool,
    /// the RequestName call waiting for its answer (0 = none)
    req: u32,
    /// the NameHasOwner(watcher) call waiting for its answer (0 = none)
    has: u32,
    revision: u32,
    /// dbusmenu id -> the row (ids from 1 in depth-first order)
    rows: Vec<(i32, TrayItem, Vec<i32>)>,
    /// server id -> our id, our id -> server id
    notes: BTreeMap<u32, u32>,
    note_srv: BTreeMap<u32, u32>,
    /// Notify call serial -> our id (until the answer)
    pending: BTreeMap<u32, u32>,
    clicked: Vec<u32>,
}

fn service() -> String {
    // SAFETY: getpid has no arguments.
    alloc::format!("org.kde.StatusNotifierItem-{}-1", unsafe { syscall6(39, 0, 0, 0, 0, 0, 0) })
}

fn run() {
    let Some(mut c) = Conn::session() else {
        STATE.store(2, Ordering::Release);
        return;
    };
    let rule = |r: &str| Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", alloc::vec![Value::str(r)]);
    c.send(&rule("type='signal',interface='org.freedesktop.Notifications'"));
    c.send(&rule("type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.kde.StatusNotifierWatcher'"));
    // (after the match: a watcher that starts meanwhile is seen either way)
    let has = c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", alloc::vec![Value::str(WATCHER)]));
    if has.is_none() {
        WATCHER_UP.store(0, Ordering::Release);
    }
    let mut it = Item { tray: None, named: false, own_name: false, req: 0, has: has.unwrap_or(0), revision: 1, rows: Vec::new(), notes: BTreeMap::new(), note_srv: BTreeMap::new(), pending: BTreeMap::new(), clicked: Vec::new() };
    loop {
        let ops = core::mem::take(&mut *OPS.lock());
        for op in ops {
            apply(&mut c, &mut it, op);
        }
        while let Some(m) = c.read(0) {
            handle(&mut c, &mut it, &m);
        }
        if c.broken {
            STATE.store(2, Ordering::Release);
            return;
        }
        // wait for the bus or the eventfd
        let wake = WAKE.load(Ordering::Acquire);
        let mut pfd = [c.fd() as u32 as u64 | (1u64 << 32), wake as u32 as u64 | (1u64 << 32)];
        // SAFETY: two pollfd records.
        unsafe { syscall6(7, pfd.as_mut_ptr() as usize, 2, usize::MAX, 0, 0, 0) };
        let mut n = 0u64;
        // SAFETY: drain the eventfd counter (nonblocking).
        unsafe { syscall6(0, wake as usize, &mut n as *mut u64 as usize, 8, 0, 0, 0) };
    }
}

fn apply(c: &mut Conn, it: &mut Item, op: Op) {
    match op {
        Op::Set(t) => {
            it.tray = Some(t);
            it.revision = it.revision.wrapping_add(1);
            it.rows.clear();
            let menu = it.tray.as_ref().map(|t| t.menu.clone()).unwrap_or_default();
            number(&menu, &mut it.rows, &mut 1);
            if it.named {
                for member in ["NewIcon", "NewToolTip", "NewTitle"] {
                    c.send(&Msg::signal(SNI_PATH, SNI_IFACE, member, Vec::new()));
                }
                c.send(&Msg::signal(MENU_PATH, MENU_IFACE, "LayoutUpdated", alloc::vec![Value::U(it.revision), Value::I(0)]));
            } else {
                announce(c, it);
            }
        }
        Op::Remove => {
            it.tray = None;
            if it.named {
                if it.own_name {
                    c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ReleaseName", alloc::vec![Value::S(service())]));
                } else {
                    // (known by the connection's name: the watcher sees a Passive item)
                    c.send(&Msg::signal(SNI_PATH, SNI_IFACE, "NewStatus", alloc::vec![Value::str("Passive")]));
                }
                it.named = false;
                it.own_name = false;
            }
        }
        Op::Notify(id, n) => {
            let mut hints = alloc::vec![("desktop-entry", Value::S(crate::linux_desk::desktop_id()))];
            if let Some(i) = &n.icon {
                hints.push(("image-data", image_data(i)));
            }
            let body = alloc::vec![
                Value::S(crate::vfs::app_id()),
                Value::U(0),
                Value::S(if n.icon.is_some() { String::new() } else { crate::linux_desk::desktop_id() }),
                Value::S(n.title),
                Value::S(n.body),
                Value::A(String::from("s"), alloc::vec![Value::str("default"), Value::str("Öffnen")]),
                Value::dict(hints),
                Value::I(-1),
            ];
            if let Some(serial) = c.send(&Msg::call(NOTES, "/org/freedesktop/Notifications", NOTES, "Notify", body)) {
                it.pending.insert(serial, id);
            }
        }
        Op::Close(id) => {
            if let Some(&srv) = it.note_srv.get(&id) {
                c.send(&Msg::call(NOTES, "/org/freedesktop/Notifications", NOTES, "CloseNotification", alloc::vec![Value::U(srv)]));
            }
        }
    }
}

// take the name; its answer tells the watcher about the item (register)
fn announce(c: &mut Conn, it: &mut Item) {
    let name = service();
    let r = c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName", alloc::vec![Value::S(name), Value::U(4)]));
    it.req = r.unwrap_or(0);
    it.named = true;
    if it.req == 0 {
        register(c, it);
    }
}

// RegisterStatusNotifierItem: by the name service() when it is ours, else by the object path
// (the watcher takes the caller's own name with it)
fn register(c: &mut Conn, it: &Item) {
    let id = if it.own_name { service() } else { String::from(SNI_PATH) };
    c.send(&Msg::call(WATCHER, "/StatusNotifierWatcher", WATCHER, "RegisterStatusNotifierItem", alloc::vec![Value::S(id)]));
}

// dbusmenu ids for the rows, depth first; each entry lists its children's ids
fn number(items: &[TrayItem], rows: &mut Vec<(i32, TrayItem, Vec<i32>)>, next: &mut i32) -> Vec<i32> {
    let mut ids = Vec::new();
    for item in items {
        let id = *next;
        *next += 1;
        let at = rows.len();
        rows.push((id, item.clone(), Vec::new()));
        let kids = number(&item.sub, rows, next);
        rows[at].2 = kids;
        ids.push(id);
    }
    ids
}

// "&Datei" (the menus' mnemonic) -> "_Datei"; "&&" -> "&", "_" -> "__"
fn dbus_label(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(ch) = it.next() {
        match ch {
            '&' if it.peek() == Some(&'&') => {
                it.next();
                out.push('&');
            }
            '&' => out.push('_'),
            '_' => out.push_str("__"),
            _ => out.push(ch),
        }
    }
    out
}

fn props(item: &TrayItem) -> Value {
    let mut p = Vec::new();
    if item.flags & TI_SEPARATOR != 0 {
        p.push(("type", Value::str("separator")));
    } else {
        p.push(("label", Value::S(dbus_label(&item.label))));
        p.push(("enabled", Value::B(item.flags & TI_DISABLED == 0)));
        if item.flags & TI_CHECKED != 0 {
            p.push(("toggle-type", Value::str("checkmark")));
            p.push(("toggle-state", Value::I(1)));
        }
        if !item.sub.is_empty() {
            p.push(("children-display", Value::str("submenu")));
        }
    }
    Value::dict(p)
}

// (ia{sv}av) of a row and its children
fn layout(it: &Item, id: i32, depth: i32) -> Value {
    let (p, kids) = if id == 0 {
        let top: Vec<i32> = top_ids(it);
        (Value::dict(alloc::vec![("children-display", Value::str("submenu"))]), top)
    } else {
        match it.rows.iter().find(|r| r.0 == id) {
            Some(r) => (props(&r.1), r.2.clone()),
            None => (Value::dict(Vec::new()), Vec::new()),
        }
    };
    let children = if depth == 0 { Vec::new() } else { kids.iter().map(|&k| Value::var(layout(it, k, depth - 1))).collect() };
    Value::R(alloc::vec![Value::I(id), p, Value::A(String::from("v"), children)])
}

fn top_ids(it: &Item) -> Vec<i32> {
    let mut all_children: Vec<i32> = Vec::new();
    for r in &it.rows {
        all_children.extend_from_slice(&r.2);
    }
    it.rows.iter().map(|r| r.0).filter(|id| !all_children.contains(id)).collect()
}

fn icon_pixmaps(i: &TrayIcon) -> Value {
    let mut bytes = Vec::with_capacity(i.px.len() * 4);
    for &p in &i.px {
        bytes.extend_from_slice(&p.to_be_bytes()); // ARGB32 in network byte order
    }
    let data = Value::A(String::from("y"), bytes.into_iter().map(Value::Y).collect());
    Value::A(String::from("(iiay)"), alloc::vec![Value::R(alloc::vec![Value::I(i.w as i32), Value::I(i.h as i32), data])])
}

// (iiibiiay) RGBA for the notification's image-data hint
fn image_data(i: &TrayIcon) -> Value {
    let mut bytes = Vec::with_capacity(i.px.len() * 4);
    for &p in &i.px {
        bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
    }
    Value::R(alloc::vec![
        Value::I(i.w as i32),
        Value::I(i.h as i32),
        Value::I(i.w as i32 * 4),
        Value::B(true),
        Value::I(8),
        Value::I(4),
        Value::A(String::from("y"), bytes.into_iter().map(Value::Y).collect()),
    ])
}

fn sni_property(it: &Item, name: &str) -> Option<Value> {
    let t = it.tray.as_ref();
    let tooltip = t.map(|t| t.tooltip.clone()).unwrap_or_default();
    let empty_pixmaps = || Value::A(String::from("(iiay)"), Vec::new());
    Some(match name {
        "Category" => Value::str("ApplicationStatus"),
        "Id" => Value::S(crate::vfs::app_id()),
        "Title" => Value::S(if tooltip.is_empty() { crate::vfs::app_id() } else { tooltip.clone() }),
        "Status" => Value::str(if t.is_some() { "Active" } else { "Passive" }),
        "WindowId" => Value::I(0),
        "IconName" => Value::S(if t.and_then(|t| t.icon.as_ref()).is_some() { String::new() } else { crate::linux_desk::desktop_id() }),
        "IconPixmap" => t.and_then(|t| t.icon.as_ref()).map_or_else(empty_pixmaps, icon_pixmaps),
        "OverlayIconName" | "AttentionIconName" | "AttentionMovieName" | "IconThemePath" => Value::str(""),
        "OverlayIconPixmap" | "AttentionIconPixmap" => empty_pixmaps(),
        "ToolTip" => Value::R(alloc::vec![Value::str(""), empty_pixmaps(), Value::S(tooltip), Value::str("")]),
        "ItemIsMenu" => Value::B(false),
        "Menu" => Value::O(String::from(MENU_PATH)),
        _ => return None,
    })
}

const SNI_PROPS: [&str; 16] = [
    "Category", "Id", "Title", "Status", "WindowId", "IconName", "IconPixmap", "OverlayIconName", "OverlayIconPixmap",
    "AttentionIconName", "AttentionIconPixmap", "AttentionMovieName", "IconThemePath", "ToolTip", "ItemIsMenu", "Menu",
];

fn menu_property(name: &str) -> Option<Value> {
    Some(match name {
        "Version" => Value::U(3),
        "TextDirection" => Value::str("ltr"),
        "Status" => Value::str("normal"),
        "IconThemePath" => Value::A(String::from("s"), Vec::new()),
        _ => return None,
    })
}

fn introspection(path: &str) -> String {
    let body = match path {
        SNI_PATH => "<interface name=\"org.kde.StatusNotifierItem\"><method name=\"Activate\"><arg type=\"i\" direction=\"in\"/><arg type=\"i\" direction=\"in\"/></method><method name=\"SecondaryActivate\"><arg type=\"i\" direction=\"in\"/><arg type=\"i\" direction=\"in\"/></method><method name=\"ContextMenu\"><arg type=\"i\" direction=\"in\"/><arg type=\"i\" direction=\"in\"/></method><method name=\"Scroll\"><arg type=\"i\" direction=\"in\"/><arg type=\"s\" direction=\"in\"/></method><method name=\"ProvideXdgActivationToken\"><arg type=\"s\" direction=\"in\"/></method><signal name=\"NewIcon\"/><signal name=\"NewToolTip\"/><signal name=\"NewTitle\"/><signal name=\"NewStatus\"><arg type=\"s\"/></signal></interface>",
        MENU_PATH => "<interface name=\"com.canonical.dbusmenu\"><method name=\"GetLayout\"><arg type=\"i\" direction=\"in\"/><arg type=\"i\" direction=\"in\"/><arg type=\"as\" direction=\"in\"/><arg type=\"u\" direction=\"out\"/><arg type=\"(ia{sv}av)\" direction=\"out\"/></method><method name=\"Event\"><arg type=\"i\" direction=\"in\"/><arg type=\"s\" direction=\"in\"/><arg type=\"v\" direction=\"in\"/><arg type=\"u\" direction=\"in\"/></method><method name=\"AboutToShow\"><arg type=\"i\" direction=\"in\"/><arg type=\"b\" direction=\"out\"/></method><signal name=\"LayoutUpdated\"><arg type=\"u\"/><arg type=\"i\"/></signal></interface>",
        _ => "",
    };
    alloc::format!("<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\" \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\"><node>{body}</node>")
}

fn arg_i(m: &Msg, k: usize) -> i32 {
    match m.body.get(k) {
        Some(Value::I(v)) => *v,
        Some(v) => v.as_u32().unwrap_or(0) as i32,
        None => 0,
    }
}

fn arg_s(m: &Msg, k: usize) -> &str {
    m.body.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

fn handle(c: &mut Conn, it: &mut Item, m: &Msg) {
    match m.typ {
        T_RETURN | T_ERROR if it.has != 0 && m.reply_to == it.has => {
            it.has = 0;
            let up = m.typ == T_RETURN && matches!(m.body.first(), Some(Value::B(true)));
            WATCHER_UP.store(up as i32, Ordering::Release);
        }
        T_RETURN | T_ERROR if it.req != 0 && m.reply_to == it.req => {
            // RequestName: 1 primary owner, 4 already the owner
            it.req = 0;
            it.own_name = m.typ == T_RETURN && matches!(m.body.first().and_then(|v| v.as_u32()), Some(1 | 4));
            if it.named {
                register(c, it);
            }
        }
        T_RETURN | T_ERROR => {
            if let Some(id) = it.pending.remove(&m.reply_to) {
                if let Some(srv) = m.body.first().and_then(|v| v.as_u32()).filter(|_| m.typ == T_RETURN) {
                    it.notes.insert(srv, id);
                    it.note_srv.insert(id, srv);
                }
            }
        }
        T_SIGNAL => match (m.iface.as_str(), m.member.as_str()) {
            (NOTES, "ActivationToken") => {
                // (spec 1.2) before ActionInvoked: the window the app raises may come forward
                if m.body.first().and_then(|v| v.as_u32()).is_some_and(|s| it.notes.contains_key(&s)) {
                    crate::act_provide(arg_s(m, 1));
                }
            }
            (NOTES, "ActionInvoked") => {
                if let Some(&id) = m.body.first().and_then(|v| v.as_u32()).and_then(|s| it.notes.get(&s)) {
                    it.clicked.push(id);
                    tray::push(TrayEvent::NoteClicked(id));
                }
            }
            (NOTES, "NotificationClosed") => {
                if let Some(srv) = m.body.first().and_then(|v| v.as_u32()) {
                    if let Some(id) = it.notes.remove(&srv) {
                        it.note_srv.remove(&id);
                        if let Some(k) = it.clicked.iter().position(|&x| x == id) {
                            it.clicked.remove(k);
                        } else {
                            tray::push(TrayEvent::NoteClosed(id));
                        }
                    }
                }
            }
            ("org.freedesktop.DBus", "NameOwnerChanged") if arg_s(m, 0) == WATCHER => {
                // the watcher came or went (supported()); started anew: announce the item again
                let up = !arg_s(m, 2).is_empty();
                WATCHER_UP.store(up as i32, Ordering::Release);
                if up && it.tray.is_some() {
                    if it.named && it.req == 0 {
                        register(c, it);
                    } else if !it.named {
                        announce(c, it);
                    }
                }
            }
            _ => {}
        },
        T_CALL => {
            let r = call(it, m);
            if m.flags & crate::linux_dbus::F_NO_REPLY == 0 {
                c.send(&match r {
                    Some(body) => Msg::reply(m, body),
                    None => Msg::error(m, "org.freedesktop.DBus.Error.UnknownMethod", "unknown method"),
                });
            }
        }
        _ => {}
    }
}

// the answer body of a call to one of our objects (None: unknown)
fn call(it: &Item, m: &Msg) -> Option<Vec<Value>> {
    match (m.path.as_str(), m.iface.as_str(), m.member.as_str()) {
        (_, "org.freedesktop.DBus.Peer", "Ping") => Some(Vec::new()),
        (p, "org.freedesktop.DBus.Introspectable", "Introspect") => Some(alloc::vec![Value::S(introspection(p))]),
        (SNI_PATH, "org.freedesktop.DBus.Properties", "Get") => sni_property(it, arg_s(m, 1)).map(|v| alloc::vec![Value::var(v)]),
        (SNI_PATH, "org.freedesktop.DBus.Properties", "GetAll") => {
            Some(alloc::vec![Value::dict(SNI_PROPS.iter().filter_map(|n| sni_property(it, n).map(|v| (*n, v))).collect())])
        }
        (SNI_PATH, SNI_IFACE, "Activate") => {
            tray::push(TrayEvent::Activate);
            Some(Vec::new())
        }
        (SNI_PATH, SNI_IFACE, "SecondaryActivate") => {
            tray::push(TrayEvent::SecondaryActivate);
            Some(Vec::new())
        }
        (SNI_PATH, SNI_IFACE, "ProvideXdgActivationToken") => {
            crate::act_provide(arg_s(m, 0));
            Some(Vec::new())
        }
        (SNI_PATH, SNI_IFACE, "ContextMenu" | "Scroll") => Some(Vec::new()),
        (MENU_PATH, "org.freedesktop.DBus.Properties", "Get") => menu_property(arg_s(m, 1)).map(|v| alloc::vec![Value::var(v)]),
        (MENU_PATH, "org.freedesktop.DBus.Properties", "GetAll") => Some(alloc::vec![Value::dict(
            ["Version", "TextDirection", "Status", "IconThemePath"].iter().filter_map(|n| menu_property(n).map(|v| (*n, v))).collect()
        )]),
        (MENU_PATH, MENU_IFACE, "GetLayout") => {
            let depth = arg_i(m, 1);
            Some(alloc::vec![Value::U(it.revision), layout(it, arg_i(m, 0), if depth < 0 { 64 } else { depth })])
        }
        (MENU_PATH, MENU_IFACE, "GetGroupProperties") => {
            let ids: Vec<i32> = match m.body.first() {
                Some(Value::A(_, v)) => v.iter().map(|x| x.as_u32().unwrap_or(0) as i32).collect(),
                _ => Vec::new(),
            };
            let rows = it.rows.iter().filter(|r| ids.is_empty() || ids.contains(&r.0)).map(|r| Value::R(alloc::vec![Value::I(r.0), props(&r.1)])).collect();
            Some(alloc::vec![Value::A(String::from("(ia{sv})"), rows)])
        }
        (MENU_PATH, MENU_IFACE, "GetProperty") => Some(alloc::vec![Value::var(Value::str(""))]),
        (MENU_PATH, MENU_IFACE, "Event") => {
            menu_event(it, arg_i(m, 0), arg_s(m, 1));
            Some(Vec::new())
        }
        (MENU_PATH, MENU_IFACE, "EventGroup") => {
            if let Some(Value::A(_, evs)) = m.body.first() {
                for e in evs {
                    if let Value::R(f) = e {
                        let id = f.first().and_then(|v| v.as_u32()).unwrap_or(0) as i32;
                        menu_event(it, id, f.get(1).and_then(|v| v.as_str()).unwrap_or(""));
                    }
                }
            }
            Some(alloc::vec![Value::A(String::from("i"), Vec::new())])
        }
        (MENU_PATH, MENU_IFACE, "AboutToShow") => Some(alloc::vec![Value::B(false)]),
        (MENU_PATH, MENU_IFACE, "AboutToShowGroup") => {
            Some(alloc::vec![Value::A(String::from("i"), Vec::new()), Value::A(String::from("i"), Vec::new())])
        }
        _ => None,
    }
}

fn menu_event(it: &Item, id: i32, event: &str) {
    if event != "clicked" {
        return;
    }
    if let Some(r) = it.rows.iter().find(|r| r.0 == id) {
        if r.1.id != 0 && r.1.flags & (TI_DISABLED | TI_SEPARATOR) == 0 && r.1.sub.is_empty() {
            tray::push(TrayEvent::Menu(r.1.id));
        }
    }
}
