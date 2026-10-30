// win_assoc -- (assoc.rs) the file types and URL schemes below HKCU\Software\Classes
//
//   available()           Unavailable in an MSIX package (its manifest declares them)
//   conflicts(pkg, exe, mine)   what register would ask about, as if the record mine were undone
//   register(pkg, exe, take, lines, done)   reg_entries written; the record lines:
//                         key <root>            a key of the app's own created (removed as a whole)
//                         mkkey <key>           a shared key created (removed when empty)
//                         val <key> <name> <v>  a value added (removed while it is still v)
//                         old <key> <name> <type> <hex> <v>   a value replaced (put back while
//                                               it is still v)
//                         bakkey <key> / bak <key> <name> <type> <hex>   a key of another
//                                               program taken over: what it held (put back
//                                               after its replacement is removed)
//   undo(lines)           the lines backwards
//   changed(notes)        SHChangeNotify(SHCNE_ASSOCCHANGED)
//   console_print(text)   no stdout: AttachConsole(ATTACH_PARENT_PROCESS) + WriteConsoleW
//
// A key of the app's own (ProgID <app>.<ext>, a scheme) is the program's when its
// shell\open\command is the command written here; with another command (in HKCU or HKLM) it
// belongs to another program (another copy of the app too).  The default of an extension
// belongs to another program when HKCR's (HKCU over HKLM) names another ProgID; a UserChoice
// (the user's pick in Windows' dialog, hashed) stays as it is.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{AssocError, Conflict, Line, Package, Registered, RegValue, RV_DEFAULT, RV_SHARED, RV_TREE, reg_entries};

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: isize, sub: *const u16, opts: u32, sam: u32, out: *mut isize) -> i32;
    fn RegCreateKeyExW(key: isize, sub: *const u16, res: u32, class: usize, opts: u32, sam: u32, sec: usize, out: *mut isize, disp: *mut u32) -> i32;
    fn RegCloseKey(key: isize) -> i32;
    fn RegGetValueW(key: isize, sub: *const u16, name: *const u16, flags: u32, typ: *mut u32, data: *mut u8, n: *mut u32) -> i32;
    fn RegSetKeyValueW(key: isize, sub: *const u16, name: *const u16, typ: u32, data: *const u8, n: u32) -> i32;
    fn RegDeleteKeyValueW(key: isize, sub: *const u16, name: *const u16) -> i32;
    fn RegDeleteTreeW(key: isize, sub: *const u16) -> i32;
    fn RegDeleteKeyW(key: isize, sub: *const u16) -> i32;
    fn RegQueryInfoKeyW(key: isize, class: usize, classlen: usize, res: usize, subkeys: *mut u32, maxsub: *mut u32, maxclass: usize,
                        values: *mut u32, maxname: *mut u32, maxdata: *mut u32, sec: usize, time: usize) -> i32;
    fn RegEnumKeyExW(key: isize, i: u32, name: *mut u16, n: *mut u32, res: usize, class: usize, classlen: usize, time: usize) -> i32;
    fn RegEnumValueW(key: isize, i: u32, name: *mut u16, n: *mut u32, res: usize, typ: *mut u32, data: *mut u8, len: *mut u32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetStdHandle(n: u32) -> isize;
    fn AttachConsole(pid: u32) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sec: usize, disp: u32, flags: u32, tmpl: isize) -> isize;
    fn WriteConsoleW(h: isize, buf: *const u16, n: u32, done: *mut u32, res: usize) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

const HKCU: isize = 0xFFFF_FFFF_8000_0001u64 as isize;
const HKLM: isize = 0xFFFF_FFFF_8000_0002u64 as isize;
const KEY_READ: u32 = 0x2_0019;
const KEY_WRITE: u32 = 0x2_0006;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const RRF_RT_ANY: u32 = 0xFFFF;
const RRF_NOEXPAND: u32 = 0x1000_0000;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const CLASSES: &str = "Software\\Classes";
const FILE_EXTS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts";
const URL_ASSOC: &str = "Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn from_wide(w: &[u16]) -> String {
    String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn unhex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    b.chunks_exact(2).filter_map(|p| Some(super::hexval(p[0])? << 4 | super::hexval(p[1])?)).collect()
}

// ---------------------------------------------------------------- the registry

/// an open key, closed when dropped
struct Key(isize);

impl Key {
    fn open(root: isize, sub: &str, sam: u32) -> Option<Key> {
        let w = wide(sub);
        let mut h = 0isize;
        // SAFETY: NUL-terminated name, h receives the handle.
        (unsafe { RegOpenKeyExW(root, w.as_ptr(), 0, sam, &mut h) } == 0).then_some(Key(h))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: a handle RegOpenKeyExW / RegCreateKeyExW returned.
        unsafe { RegCloseKey(self.0) };
    }
}

fn exists(root: isize, sub: &str) -> bool {
    Key::open(root, sub, KEY_READ).is_some()
}

/// a value as (type, bytes); None: missing
fn get_raw(root: isize, sub: &str, name: &str) -> Option<(u32, Vec<u8>)> {
    let (ws, wn) = (wide(sub), wide(name));
    let (mut typ, mut n) = (0u32, 0u32);
    let flags = RRF_RT_ANY | RRF_NOEXPAND;
    // SAFETY: NUL-terminated names; first the size, then a buffer of that size (retried once
    // when the value grew meanwhile).
    unsafe {
        if RegGetValueW(root, ws.as_ptr(), wn.as_ptr(), flags, &mut typ, core::ptr::null_mut(), &mut n) != 0 {
            return None;
        }
        for _ in 0..2 {
            let mut buf = alloc::vec![0u8; n as usize + 2];
            let mut m = buf.len() as u32;
            if RegGetValueW(root, ws.as_ptr(), wn.as_ptr(), flags, &mut typ, buf.as_mut_ptr(), &mut m) == 0 {
                buf.truncate(m as usize);
                return Some((typ, buf));
            }
            n = m;
        }
    }
    None
}

fn sz_of(typ: u32, data: &[u8]) -> Option<String> {
    if typ != REG_SZ && typ != REG_EXPAND_SZ {
        return None;
    }
    let w: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(from_wide(&w))
}

/// a string value (REG_SZ / REG_EXPAND_SZ, not expanded)
fn get_sz(root: isize, sub: &str, name: &str) -> Option<String> {
    let (t, d) = get_raw(root, sub, name)?;
    sz_of(t, &d)
}

fn set_raw(sub: &str, name: &str, typ: u32, data: &[u8]) -> bool {
    let (ws, wn) = (wide(sub), wide(name));
    // SAFETY: NUL-terminated names, data.len() bytes (RegSetKeyValueW creates the key).
    unsafe { RegSetKeyValueW(HKCU, ws.as_ptr(), wn.as_ptr(), typ, data.as_ptr(), data.len() as u32) == 0 }
}

fn set_sz(sub: &str, name: &str, value: &str) -> bool {
    let w = wide(value);
    let bytes: Vec<u8> = w.iter().flat_map(|c| c.to_le_bytes()).collect();
    set_raw(sub, name, REG_SZ, &bytes)
}

fn create_key(sub: &str) -> bool {
    let w = wide(sub);
    let mut h = 0isize;
    let mut disp = 0u32;
    // SAFETY: NUL-terminated name; the handle is closed by Key's drop.
    let ok = unsafe { RegCreateKeyExW(HKCU, w.as_ptr(), 0, 0, 0, KEY_WRITE, 0, &mut h, &mut disp) } == 0;
    if ok {
        drop(Key(h));
    }
    ok
}

fn del_value(sub: &str, name: &str) -> bool {
    let (ws, wn) = (wide(sub), wide(name));
    // SAFETY: NUL-terminated names.
    let r = unsafe { RegDeleteKeyValueW(HKCU, ws.as_ptr(), wn.as_ptr()) };
    r == 0 || r == ERROR_FILE_NOT_FOUND
}

fn del_tree(sub: &str) -> bool {
    // only below Software\Classes: a damaged record line (an empty key name would clear HKCU
    // itself) must not reach further
    let b = sub.as_bytes();
    if b.len() <= CLASSES.len() + 1 || !b[..CLASSES.len()].eq_ignore_ascii_case(CLASSES.as_bytes()) || b[CLASSES.len()] != b'\\' {
        return false;
    }
    let w = wide(sub);
    // SAFETY: NUL-terminated name; RegDeleteTreeW leaves the (then empty) key, RegDeleteKeyW
    // removes it.
    unsafe {
        let r = RegDeleteTreeW(HKCU, w.as_ptr());
        (r == 0 || r == ERROR_FILE_NOT_FOUND) && matches!(RegDeleteKeyW(HKCU, w.as_ptr()), 0 | ERROR_FILE_NOT_FOUND)
    }
}

/// (subkeys, values) of HKCU\sub
fn counts(sub: &str) -> Option<(u32, u32)> {
    let k = Key::open(HKCU, sub, KEY_READ)?;
    let (mut subs, mut vals) = (0u32, 0u32);
    let (mut a, mut b, mut c) = (0u32, 0u32, 0u32);
    // SAFETY: the key is open; only the counts and the maxima are written.
    let r = unsafe { RegQueryInfoKeyW(k.0, 0, 0, 0, &mut subs, &mut a, 0, &mut vals, &mut b, &mut c, 0, 0) };
    (r == 0).then_some((subs, vals))
}

/// everything below HKCU\sub as record lines: bakkey (every key), bak (every value)
fn dump(sub: &str, out: &mut Vec<Line>) {
    out.push(alloc::vec![String::from("bakkey"), String::from(sub)]);
    let Some(k) = Key::open(HKCU, sub, KEY_READ) else { return };
    let mut i = 0u32;
    loop {
        let mut name = [0u16; 16384];
        let mut n = name.len() as u32;
        let mut typ = 0u32;
        let mut len = 0u32;
        // SAFETY: the key is open; first the name and the data's size.
        let r = unsafe { RegEnumValueW(k.0, i, name.as_mut_ptr(), &mut n, 0, &mut typ, core::ptr::null_mut(), &mut len) };
        if r != 0 {
            break;
        }
        let vname = from_wide(&name[..n as usize]);
        if let Some((t, d)) = get_raw(HKCU, sub, &vname) {
            out.push(alloc::vec![String::from("bak"), String::from(sub), vname, format!("{t}"), hex(&d)]);
        }
        i += 1;
    }
    let mut j = 0u32;
    let mut children = Vec::new();
    loop {
        let mut name = [0u16; 256];
        let mut n = name.len() as u32;
        // SAFETY: the key is open; name holds 255 characters + NUL (the limit of key names).
        if unsafe { RegEnumKeyExW(k.0, j, name.as_mut_ptr(), &mut n, 0, 0, 0, 0) } != 0 {
            break;
        }
        children.push(format!("{sub}\\{}", from_wide(&name[..n as usize])));
        j += 1;
    }
    drop(k);
    for c in children {
        dump(&c, out);
    }
}

// ---------------------------------------------------------------- the state before

/// HKCU as it was before the record mine (assoc_conflicts while registered)
struct View<'a> {
    mine: &'a [Line],
}

impl View<'_> {
    fn created(&self, sub: &str) -> bool {
        let low = sub.to_ascii_lowercase();
        self.mine.iter().any(|l| {
            matches!(l[0].as_str(), "key" | "mkkey") && l.len() > 1 && {
                let k = l[1].to_ascii_lowercase();
                low == k || low.starts_with(&format!("{k}\\"))
            }
        })
    }

    fn exists(&self, sub: &str) -> bool {
        !self.created(sub) && exists(HKCU, sub)
    }

    /// a string value of HKCU
    fn get(&self, sub: &str, name: &str) -> Option<String> {
        if self.created(sub) {
            return None;
        }
        for l in self.mine {
            if l.len() >= 3 && l[1].eq_ignore_ascii_case(sub) && l[2].eq_ignore_ascii_case(name) {
                match l[0].as_str() {
                    "val" => return None,
                    "old" if l.len() >= 5 => return sz_of(l[3].parse().unwrap_or(0), &unhex(&l[4])),
                    _ => {}
                }
            }
        }
        get_sz(HKCU, sub, name)
    }

    /// HKCU over HKLM (the HKCR view)
    fn get_classes(&self, sub: &str, name: &str) -> Option<String> {
        if self.exists(sub) || self.mine.iter().any(|l| l.len() >= 3 && l[0] == "old" && l[1].eq_ignore_ascii_case(sub)) {
            return self.get(sub, name);
        }
        get_sz(HKLM, sub, name)
    }
}

/// the state of a key of the app's own (a ProgID, a scheme)
enum Tree {
    /// missing (or with this program's command)
    Free,
    Mine,
    /// another program's: its command, and whether it is in HKCU (else HKLM)
    Other(String, bool),
}

fn tree_state(v: &View, root: &str, cmd: &str) -> Tree {
    let command = format!("{root}\\shell\\open\\command");
    if v.exists(root) {
        return match v.get(&command, "") {
            Some(c) if c.eq_ignore_ascii_case(cmd) => Tree::Mine,
            c => Tree::Other(c.unwrap_or_else(|| String::from("(no command)")), true),
        };
    }
    if exists(HKLM, root) {
        return match get_sz(HKLM, &command, "") {
            Some(c) if c.eq_ignore_ascii_case(cmd) => Tree::Free,
            c => Tree::Other(c.unwrap_or_else(|| String::from("(no command)")), false),
        };
    }
    Tree::Free
}

/// the user's pick for an extension / a scheme in Windows' dialog (UserChoice ProgId)
fn user_choice(what: &str) -> Option<String> {
    let key = match what.strip_prefix('.') {
        Some(_) => format!("{FILE_EXTS}\\{what}\\UserChoice"),
        None => format!("{URL_ASSOC}\\{}\\UserChoice", what.trim_end_matches(':')),
    };
    get_sz(HKCU, &key, "ProgId").filter(|p| !p.is_empty())
}

fn command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

fn label(root: &str) -> String {
    let name = &root[CLASSES.len() + 1..];
    name.to_ascii_lowercase()
}

/// the decisions of a registration: (root, Some(conflict) | None) and (.ext key, conflict)
fn plan(pkg: &Package, exe: &str, v: &View) -> (Vec<RegValue>, Vec<(String, Option<Conflict>)>, Vec<(String, Conflict)>) {
    let entries = reg_entries(&pkg.app, &pkg.display, exe, &pkg.types, &pkg.schemes, &pkg.app_id);
    let cmd = command(exe);
    let mut roots: Vec<(String, Option<Conflict>)> = Vec::new();
    let mut defaults: Vec<(String, Conflict)> = Vec::new();
    for e in &entries {
        if e.kind == RV_TREE && !roots.iter().any(|(r, _)| *r == e.root) {
            let is_scheme = pkg.schemes.iter().any(|s| e.root.eq_ignore_ascii_case(&format!("{CLASSES}\\{s}")));
            let what = if is_scheme { format!("{}:", label(&e.root)) } else { String::from(&e.root[CLASSES.len() + 1..]) };
            let c = match tree_state(v, &e.root, &cmd) {
                Tree::Other(holder, _) => Some(Conflict { what: what.clone(), holder, fixed: false }),
                _ => None,
            };
            let c = c.or_else(|| {
                let p = user_choice(&what).filter(|_| is_scheme)?;
                (!p.eq_ignore_ascii_case(&label(&e.root))).then(|| Conflict { what, holder: p, fixed: true })
            });
            roots.push((e.root.clone(), c));
        }
        if e.kind == RV_DEFAULT && e.name.is_empty() && !defaults.iter().any(|(k, _)| *k == e.key) {
            let ext = String::from(&e.key[CLASSES.len() + 1..]);
            if let Some(p) = user_choice(&ext).filter(|p| !p.eq_ignore_ascii_case(&e.value)) {
                defaults.push((e.key.clone(), Conflict { what: ext, holder: p, fixed: true }));
            } else if let Some(p) = v.get_classes(&e.key, "").filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case(&e.value)) {
                defaults.push((e.key.clone(), Conflict { what: ext, holder: p, fixed: false }));
            }
        }
    }
    (entries, roots, defaults)
}

pub fn available() -> Result<(), AssocError> {
    if crate::autostart::native::packaged() {
        return Err(AssocError::Unavailable);
    }
    Ok(())
}

pub fn conflicts(pkg: &Package, exe: &str, mine: &[Line]) -> Vec<Conflict> {
    let (_, roots, defaults) = plan(pkg, exe, &View { mine });
    roots.into_iter().filter_map(|(_, c)| c).chain(defaults.into_iter().map(|(_, c)| c)).collect()
}

/// the keys of sub below Software\Classes that do not exist yet, outermost first
fn missing_keys(sub: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut k = String::from(sub);
    while k.len() > CLASSES.len() && !exists(HKCU, &k) {
        out.push(k.clone());
        match k.rfind('\\') {
            Some(p) => k.truncate(p),
            None => break,
        }
    }
    out.reverse();
    out
}

/// write one value and record what it added or replaced
fn put(e: &RegValue, lines: &mut Vec<Line>) -> Result<(), AssocError> {
    let before = get_raw(HKCU, &e.key, &e.name);
    if before.as_ref().is_some_and(|(t, d)| sz_of(*t, d).as_deref() == Some(e.value.as_str())) {
        return Ok(());
    }
    for k in missing_keys(&e.key) {
        lines.push(alloc::vec![String::from("mkkey"), k]);
    }
    if !set_sz(&e.key, &e.name, &e.value) {
        return Err(AssocError::Failed(format!("HKCU\\{} was not written", e.key)));
    }
    lines.push(match before {
        Some((t, d)) => alloc::vec![String::from("old"), e.key.clone(), e.name.clone(), format!("{t}"), hex(&d), e.value.clone()],
        None => alloc::vec![String::from("val"), e.key.clone(), e.name.clone(), e.value.clone()],
    });
    Ok(())
}

pub fn register(pkg: &Package, exe: &str, take: &mut dyn FnMut(&Conflict) -> bool, lines: &mut Vec<Line>, done: &mut Registered) -> Result<(), AssocError> {
    let v = View { mine: &[] };
    let (entries, roots, defaults) = plan(pkg, exe, &v);
    let cmd = command(exe);
    // the decisions: keys of the app's own and extension defaults left to other programs
    let mut skip: Vec<String> = Vec::new();
    for (root, c) in &roots {
        match c {
            Some(c) if c.fixed => done.kept.push(c.clone()), // the user's pick stays, the key is written
            Some(c) if !take(c) => {
                done.kept.push(c.clone());
                skip.push(root.clone());
            }
            _ => {}
        }
    }
    for (key, c) in &defaults {
        if c.fixed || !take(c) {
            done.kept.push(c.clone());
            skip.push(key.clone());
        }
    }
    let skipped = |k: &str| skip.iter().any(|s| s.eq_ignore_ascii_case(k));
    // the keys of the app's own: created (another program's in HKCU saved and removed first)
    let mut fresh: Vec<String> = Vec::new();
    for (root, _) in roots.iter().filter(|(r, _)| !skipped(r)) {
        match tree_state(&v, root, &cmd) {
            Tree::Mine => continue,
            Tree::Other(_, true) => {
                dump(root, lines);
                if !del_tree(root) {
                    return Err(AssocError::Failed(format!("HKCU\\{root} was not replaced")));
                }
            }
            _ => {}
        }
        lines.push(alloc::vec![String::from("key"), root.clone()]);
        fresh.push(root.clone());
    }
    // the ProgID an extension's default names
    let progid_of = |key: &str| entries.iter().find(|x| x.kind == RV_DEFAULT && x.key == key && x.name.is_empty()).map(|x| x.value.clone());
    for e in &entries {
        let owner = match e.kind {
            RV_TREE => e.root.clone(),
            RV_SHARED => format!("{CLASSES}\\{}", e.name),
            _ => format!("{CLASSES}\\{}", progid_of(&e.key).unwrap_or_default()),
        };
        if skipped(&owner) || (e.kind == RV_DEFAULT && skipped(&e.key)) {
            continue;
        }
        if e.kind == RV_TREE && fresh.contains(&e.root) {
            if !set_sz(&e.key, &e.name, &e.value) {
                return Err(AssocError::Failed(format!("HKCU\\{} was not written", e.key)));
            }
        } else {
            put(e, lines)?;
        }
    }
    done.types = pkg.types.iter().filter(|t| !skipped(&format!("{CLASSES}\\{}.{}", pkg.app, t.exts[0]))).count();
    done.schemes = pkg.schemes.iter().filter(|s| !skipped(&format!("{CLASSES}\\{s}"))).count();
    Ok(())
}

pub fn undo(lines: &[Line]) {
    let field = |l: &Line, i: usize| l.get(i).cloned().unwrap_or_default();
    for l in lines.iter().rev() {
        match l[0].as_str() {
            "key" => {
                del_tree(&field(l, 1));
            }
            "mkkey" => {
                let k = field(l, 1);
                if counts(&k) == Some((0, 0)) {
                    del_tree(&k);
                }
            }
            "val" => {
                let (k, n) = (field(l, 1), field(l, 2));
                if get_sz(HKCU, &k, &n).as_deref() == Some(field(l, 3).as_str()) {
                    del_value(&k, &n);
                }
            }
            "old" => {
                let (k, n) = (field(l, 1), field(l, 2));
                if get_sz(HKCU, &k, &n).as_deref() == Some(field(l, 5).as_str()) {
                    set_raw(&k, &n, field(l, 3).parse().unwrap_or(REG_SZ), &unhex(&field(l, 4)));
                }
            }
            "bak" => {
                set_raw(&field(l, 1), &field(l, 2), field(l, 3).parse().unwrap_or(REG_SZ), &unhex(&field(l, 4)));
            }
            "bakkey" => {
                create_key(&field(l, 1));
            }
            _ => {}
        }
    }
}

pub fn changed(_notes: &mut Vec<String>) {
    crate::inst::assoc_changed();
}

/// no stdout (a GUI program in a console): the parent's console -> true when written there
pub fn console_print(text: &str) -> bool {
    // SAFETY: STD_OUTPUT_HANDLE; the handle is only compared.
    let out = unsafe { GetStdHandle(-11i32 as u32) };
    if out != 0 && out != -1 {
        return false;
    }
    let name = wide("CONOUT$");
    let w: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: ATTACH_PARENT_PROCESS; CONOUT$ opened for writing and closed again; w.len()
    // UTF-16 units written.
    unsafe {
        AttachConsole(u32::MAX);
        let h = CreateFileW(name.as_ptr(), 0x4000_0000, 2, 0, 3, 0, 0);
        if h == -1 || h == 0 {
            return false;
        }
        let mut done = 0u32;
        let ok = WriteConsoleW(h, w.as_ptr(), w.len() as u32, &mut done, 0) != 0;
        CloseHandle(h);
        ok
    }
}

/// (tests) HKCU\sub: name Some(n) its string value, None: Some("") when the key exists
pub fn hkcu_get(sub: &str, name: Option<&str>) -> Option<String> {
    match name {
        Some(n) => get_sz(HKCU, sub, n),
        None => exists(HKCU, sub).then(String::new),
    }
}
