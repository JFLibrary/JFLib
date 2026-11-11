// filedlg -- Open / Save file dialog (ContentDialog with a folder list), both OS
//                                                                  [src/ui/ui_filedlg.s]
//
//   filedlg(id, flags, open, path, cap, exts) -> EV_ACT when a file was chosen
//        flags FD_OPEN / FD_SAVE / FD_FOLDER | index << FD_EXT_SHIFT (initially selected
//        extension); FD_FOLDER chooses a folder (the own dialog lists folders only and returns
//        the selected one, else the one shown).
//        path (at most cap - 1 bytes, UTF-8) gives the initial file or folder (empty = the
//        folder used last, else the home folder) and receives the chosen path ('/'
//        separators on every OS; Windows accepts them).  exts = the extension list, one
//        entry per type: "png=PNG-Bild", "jpg;jpeg;jfif=JPEG-Bild" (lower case; several
//        extensions of one type separated by ';', the first is the one appended; optional
//        "=description"; empty = every file); FD_SAVE appends the selected type's extension
//        when the name has none.  Call it every frame like any dialog; open = 1 shows it.
//        Where the system has a file dialog (Windows, the Linux portal) that one is shown
//        (modal); Android's picker answers later: nothing is drawn while it is open, the
//        chosen document arrives as its content:// name in a later frame.  Otherwise,
//        headless or with jfgui_FILEDLG=builtin, the library's own dialog.
//        FD_MULTI (E8, with FD_OPEN): several files -- the system's dialog where it chooses
//        several (Windows, Linux's portal), the own dialog with Ctrl+click / Shift+click (or
//        Shift+arrows) on the rows ("a" "b" in the name box, quoted names typed there too);
//        path gets the first, ui.filedlg_paths() every chosen file.
//        Widget ids used: id .. id + 0x3FFF (list rows id + 17 + i).
//   filedlg_paths() -> the files of the last confirmed dialog (one without FD_MULTI)
//
// One file dialog can be open at a time (the state below is shared).

use alloc::string::String;
use alloc::vec::Vec;

use super::consts::*;
use super::i18n::tr;
use super::keys::*;
use super::win::Shared;
use super::Ui;

const FD_MAX: usize = 8192; // entries per folder
const FD_STORE: usize = 512 * 1024; // bytes of names per folder
const FD_PATH: usize = 1040;
const FD_DRIVES: usize = 256; // the plat::drives list (bytes, as the asm's buffer)
const FD_NAME_CAP: usize = 256;
const FD_DIRBIT: u32 = 0x8000_0000;

/// state shared by all windows (one file dialog at a time)
#[derive(Default)]
pub(crate) struct S {
    /// the names of the folder's entries, each ended by a NUL
    store: Vec<u8>,
    /// entries: offset in store | FD_DIRBIT for folders
    idx: Vec<u32>,
    /// the extension list of the open dialog
    exts: Vec<String>,
    msg: Option<&'static str>,
    id: u32,
    sel: i32,
    /// the system's picker answers later (plat::FD_PENDING)
    pending: bool,
    /// FD_FOLDER: folders only, a folder is the answer
    folder: bool,
    dir: String,
    last: String,
    name: String,
    /// (E8) FD_MULTI in the own dialog: the marked rows, the row a Shift+click counts from
    marks: Vec<i32>,
    anchor: i32,
    /// (E8) the files of the last confirmed dialog
    paths: Vec<String>,
}

// str_copy(src, cap): at most cap - 1 bytes, never splitting a UTF-8 sequence
fn copy_cap(src: &str, cap: usize) -> String {
    let src = super::basic::cstr(src);
    if cap == 0 {
        return String::new();
    }
    let mut n = src.len().min(cap - 1);
    while !src.is_char_boundary(n) {
        n -= 1;
    }
    String::from(&src[..n])
}

// str_cat(dst, src, cap): append, truncating like str_copy
fn cat_cap(dst: &mut String, src: &str, cap: usize) {
    if dst.len() + 1 >= cap {
        return;
    }
    let room = cap - 1 - dst.len();
    let src = super::basic::cstr(src);
    let mut n = src.len().min(room);
    while !src.is_char_boundary(n) {
        n -= 1;
    }
    dst.push_str(&src[..n]);
}

// fd_norm(s): '\' -> '/'
fn norm(s: &str) -> String {
    s.replace('\\', "/")
}

// fd_is_root(s): "/" and "X:/"
fn is_root(s: &str) -> bool {
    let b = s.as_bytes();
    match b.len() {
        1 => b[0] == b'/',
        3 => b[1] == b':' && b[2] == b'/',
        _ => false,
    }
}

// a path that names a root or starts at one ("/x", "\x", "C:...")
fn is_abs(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && (b[0] == b'/' || b[0] == b'\\' || (b.len() > 1 && b[1] == b':'))
}

// fd_canon(p): resolve "." and ".." segments and repeated '/' ('/' paths; the root "/" or
// "X:/" stays; ".." at the root is dropped)
fn canon(p: &str) -> String {
    let b = p.as_bytes();
    let mut root = 0;
    if !b.is_empty() && b[0] == b'/' {
        root = 1;
    } else if b.len() > 1 && b[1] == b':' {
        root = 2;
        if b.len() > 2 && b[2] == b'/' {
            root = 3;
        }
    }
    let mut out: Vec<u8> = b[..root].to_vec();
    let mut r = root;
    while r < b.len() {
        if b[r] == b'/' {
            r += 1;
            continue;
        }
        let mut e = r;
        while e < b.len() && b[e] != b'/' {
            e += 1;
        }
        let seg = &b[r..e];
        if seg == b"." {
            // skip
        } else if seg == b".." {
            // drop the last written segment
            while out.len() > root {
                let c = out.pop();
                if c == Some(b'/') {
                    break;
                }
            }
        } else {
            if out.len() > root {
                out.push(b'/');
            }
            out.extend_from_slice(seg);
        }
        r = e;
    }
    String::from_utf8(out).unwrap_or_default()
}

// fd_join(dst, name): dst = dst + '/' + name (FD_PATH bytes)
fn join(dst: &mut String, name: &str) {
    if !dst.is_empty() && !dst.ends_with('/') {
        cat_cap(dst, "/", FD_PATH);
    }
    cat_cap(dst, name, FD_PATH);
}

// fd_ieq(a, b): equal ignoring ASCII case; b ends at '=' or ';'
fn ieq(a: &[u8], b: &[u8]) -> bool {
    let mut i = 0;
    loop {
        let x = a.get(i).copied().unwrap_or(0).to_ascii_lowercase();
        let mut y = b.get(i).copied().unwrap_or(0);
        if y == b'=' || y == b';' {
            y = 0;
        }
        let y = y.to_ascii_lowercase();
        if x != y {
            return false;
        }
        if x == 0 {
            return true;
        }
        i += 1;
    }
}

impl S {
    // fd_entry(i) -> (name, folder) (None when out of range)
    fn entry(&self, i: i32) -> Option<(&str, bool)> {
        if i < 0 || i as usize >= self.idx.len() {
            return None;
        }
        let v = self.idx[i as usize];
        let off = (v & !FD_DIRBIT) as usize;
        let end = self.store[off..].iter().position(|&c| c == 0).map_or(self.store.len(), |n| off + n);
        Some((core::str::from_utf8(&self.store[off..end]).unwrap_or(""), v & FD_DIRBIT != 0))
    }

    // fd_ext_ok(name): the name ends in one of the extensions (or there is no filter)
    fn ext_ok(&self, name: &str) -> bool {
        if self.exts.is_empty() || self.exts[0].is_empty() {
            return true;
        }
        let Some(dot) = name.rfind('.') else { return false };
        let ext = &name.as_bytes()[dot + 1..];
        for e in &self.exts {
            if e.is_empty() {
                break;
            }
            // the extensions of this entry: "a;b;c=..."
            let b = e.as_bytes();
            let mut k = 0;
            loop {
                if ieq(ext, &b[k..]) {
                    return true;
                }
                // skip to the next one after ';' (none after '=')
                while k < b.len() && b[k] != b';' && b[k] != b'=' {
                    k += 1;
                }
                if k >= b.len() || b[k] == b'=' {
                    break;
                }
                k += 1;
            }
        }
        false
    }

    // fd_sort: folders first, then the names in the user's collation with numbers by value
    // ("Bild 2" before "Bild 10", "ä" with "a"), as the systems' file dialogs sort
    fn sort(&mut self) {
        let coll = super::i18n::collator().numeric(true);
        let store = &self.store;
        self.idx.sort_by_cached_key(|&v| {
            let off = (v & !FD_DIRBIT) as usize;
            let end = store[off..].iter().position(|&c| c == 0).map_or(store.len(), |n| off + n);
            (v & FD_DIRBIT == 0, coll.key(core::str::from_utf8(&store[off..end]).unwrap_or("")))
        });
    }

    // fd_set_dir(s): dir = s with '/' separators, no trailing separator except roots
    // (E8) FD_MULTI: row i was selected with the modifiers mods -- Ctrl adds / removes it,
    // Shift marks the files from the anchor to it, else it alone; the name box shows the
    // marked names ("a" "b" for several)
    fn mark(&mut self, i: i32, mods: u32) {
        if self.entry(i).is_none_or(|(_, dir)| dir) {
            return;
        }
        if mods & MOD_SHIFT != 0 && self.anchor >= 0 {
            let (a, b) = (self.anchor.min(i), self.anchor.max(i));
            self.marks = (a..=b).filter(|&k| self.entry(k).is_some_and(|(_, dir)| !dir)).collect();
        } else if mods & MOD_CTRL != 0 {
            if let Some(k) = self.marks.iter().position(|&m| m == i) {
                self.marks.remove(k);
            } else {
                self.marks.push(i);
            }
            self.anchor = i;
        } else {
            self.marks = alloc::vec![i];
            self.anchor = i;
        }
        let names: Vec<String> = self.marks.iter().filter_map(|&k| self.entry(k).map(|(n, _)| String::from(n))).collect();
        self.name = match names.len() {
            0 => String::new(),
            1 => copy_cap(&names[0], FD_NAME_CAP),
            _ => {
                let mut s = String::new();
                for n in &names {
                    if !s.is_empty() {
                        s.push(' ');
                    }
                    s.push('"');
                    s.push_str(n);
                    s.push('"');
                }
                s
            }
        };
        self.msg = None;
    }

    fn set_dir(&mut self, s: &str) {
        let mut d;
        if is_abs(s) {
            d = copy_cap(s, FD_PATH - 2);
        } else {
            // relative: below the current directory
            d = norm(&copy_cap(&plat::cwd(), FD_PATH - 2));
            join(&mut d, s);
        }
        let mut d = canon(&norm(&d));
        loop {
            if is_root(&d) {
                break;
            }
            let b = d.as_bytes();
            if b.len() == 2 && b[1] == b':' {
                d.push('/'); // "C:" -> "C:/"
                break;
            }
            if b.is_empty() {
                d.push('/'); // empty -> "/"
                break;
            }
            if b[b.len() - 1] != b'/' {
                break;
            }
            d.pop();
        }
        self.dir = d;
    }

    // fd_parent: go to the parent folder (roots stay)
    fn parent(&mut self) {
        if is_root(&self.dir) {
            return;
        }
        let Some(k) = self.dir.rfind('/') else { return };
        if k == 0 || (k == 2 && self.dir.as_bytes()[1] == b':') {
            self.dir.truncate(k + 1); // "/" or "C:/"
        } else {
            self.dir.truncate(k);
        }
    }

    // fd_enter(name): open the sub folder name
    fn enter(&mut self, name: &str) {
        let mut d = core::mem::take(&mut self.dir);
        join(&mut d, name);
        self.dir = d;
        self.name.clear();
    }
}

impl Ui {
    /// ui_filedlg(id, flags, open, path, cap, exts) -> EV_ACT when confirmed
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn filedlg(&mut self, id: u32, flags: u32, open: &Shared<i32>, path: &mut String, cap: u32, exts: &[&str]) -> u32 {
        let _src = self.src_scope();
        let mut result = 0;
        // first frame of an opening: load the start folder
        if open.get() != 0 && self.m.filedlg.id != id {
            // the system dialog, when there is one (modal: returns when the user is done)
            let r = self.fd_system(flags, path, cap, exts);
            if r == 1 {
                // (E8) every chosen file: the system's dialog may choose several (FD_MULTI)
                let all = plat::file_dialog_paths();
                self.m.filedlg.paths = if flags & FD_MULTI != 0 && all.len() > 1 && all[0] == *path {
                    all.iter().map(|p| copy_cap(p, cap as usize)).collect()
                } else {
                    alloc::vec![path.clone()]
                };
            }
            if r == plat::FD_PENDING {
                // the system's picker answers later (Android): wait for it
                self.m.filedlg.id = id;
                self.m.filedlg.pending = true;
            } else if r != -1 {
                open.set(0);
                if r != 0 {
                    result = EV_ACT;
                }
                self.repass();
            } else {
                self.fd_init(id, flags, path, exts);
            }
        }
        if self.m.filedlg.pending && self.m.filedlg.id == id {
            return self.fd_wait(open, path, cap);
        }
        let folder = flags & FD_FOLDER != 0;
        let title = tr(if folder { "Ordner auswählen" } else if flags & FD_SAVE != 0 { "Speichern unter" } else { "Öffnen" });
        if !self.dialog_begin(id, title, open) {
            if self.m.filedlg.id == id {
                self.m.filedlg.id = 0;
            }
            self.dialog_end();
            if result == EV_ACT {
                for p in self.m.filedlg.paths.clone() {
                    super::perm::grant_user(&p); // the system dialog: the user chose it (fs.user)
                }
            }
            return result;
        }
        self.fd_toolbar_row(id);
        // folder contents
        self.next_size(480, 300);
        let mut sel = self.m.filedlg.sel;
        let n = self.m.filedlg.idx.len() as i32;
        let (first, end) = self.list_begin(id.wrapping_add(16), &mut sel, n, 32);
        for i in first..end {
            self.fd_row(i);
        }
        let (ev, sel) = self.list_end(&mut sel);
        self.m.filedlg.sel = sel;
        if ev & EV_DOUBLE != 0 {
            if self.fd_activate(sel) && self.fd_confirm(flags, path, cap) {
                // a file was double-clicked: confirm like the primary button
                result = EV_ACT;
                open.set(0);
            }
        } else if ev & EV_ACT != 0 {
            // a file was selected: its name goes into the name box
            let multi = flags & (FD_MULTI | FD_SAVE | FD_FOLDER) == FD_MULTI;
            let mods = self.mods;
            let f = &mut self.m.filedlg;
            if multi {
                f.mark(sel, mods);
            } else if let Some((name, false)) = f.entry(sel) {
                f.name = copy_cap(name, FD_NAME_CAP);
                f.msg = None;
            }
        }
        // file name (not when choosing a folder)
        if !folder {
            self.box_begin(0, BOX_H | BOX_XCENTER, 8);
            self.text(TS_BODY, tr("Dateiname:"));
            self.next_size(-1, 0);
            let mut name = core::mem::take(&mut self.m.filedlg.name);
            let ncap = if flags & FD_MULTI != 0 { 4096 } else { FD_NAME_CAP as u32 }; // (E8) several quoted names
            let ev = self.textbox(id.wrapping_add(12), None, &mut name, ncap);
            self.m.filedlg.name = name;
            if ev & EV_ACT != 0 {
                self.m.filedlg.msg = None;
            }
            self.box_end();
        }
        if let Some(m) = self.m.filedlg.msg {
            self.text(TS_CAPTION | TX_SECONDARY, m);
        }
        let prim = tr(if folder { "Ordner auswählen" } else if flags & FD_SAVE != 0 { "Speichern" } else { "Öffnen" });
        if self.dialog_buttons(Some(prim), None, Some(tr("Abbrechen"))) == DLG_PRIMARY {
            if self.fd_confirm(flags, path, cap) {
                result = EV_ACT;
            } else {
                open.set(1); // not done yet: keep the dialog open
                self.repass();
            }
        }
        self.dialog_end();
        if result == EV_ACT {
            for p in self.m.filedlg.paths.clone() {
                super::perm::grant_user(&p); // the user chose it (fs.user)
            }
        }
        result
    }

    /// (E8) the files the last confirmed file dialog chose: with FD_MULTI every one (the first
    /// is the dialog's path), else the one; empty before any
    pub fn filedlg_paths(&self) -> Vec<String> {
        self.m.filedlg.paths.clone()
    }

    // the system's picker is open (Android): its answer, once it comes
    fn fd_wait(&mut self, open: &Shared<i32>, path: &mut String, cap: u32) -> u32 {
        let Some((r, _, chosen)) = plat::file_dialog_poll() else { return 0 };
        self.m.filedlg.pending = false;
        self.m.filedlg.id = 0;
        let wanted = open.get() != 0; // the app may have closed it meanwhile
        open.set(0);
        self.repass();
        if r != 1 || !wanted {
            return 0;
        }
        *path = copy_cap(&chosen, cap as usize);
        self.m.filedlg.paths = alloc::vec![path.clone()];
        super::perm::grant_user(path); // the user chose it (fs.user)
        EV_ACT
    }

    // fd_system(flags, path, cap, exts) -> 1 chosen, 0 cancelled, -1 use the own dialog,
    // FD_PENDING the system's picker answers later (fd_wait)
    fn fd_system(&mut self, flags: u32, path: &mut String, cap: u32, exts: &[&str]) -> i32 {
        if self.headless || plat::getenv("jfgui_FILEDLG").is_some_and(|v| v.as_bytes().first() == Some(&b'b')) {
            return -1; // "builtin"
        }
        if self.hr_outside("the system's file dialog") {
            return -1; // hot reload's replay: the own dialog (hotreload.rs)
        }
        let r = plat::file_dialog(flags, path, exts).0;
        if r == 1 {
            *path = copy_cap(path, cap as usize);
        }
        r
    }

    // fd_init(id, flags, path, exts): the dialog opens: choose the start folder and name, list it
    fn fd_init(&mut self, id: u32, flags: u32, path: &str, exts: &[&str]) {
        let f = &mut self.m.filedlg;
        f.id = id;
        f.folder = flags & FD_FOLDER != 0;
        f.exts = exts.iter().map(|e| String::from(*e)).collect();
        f.name.clear();
        f.msg = None;
        let path = super::basic::cstr(path);
        if !path.is_empty() {
            let mut tmp = norm(&copy_cap(path, FD_PATH));
            if super::perm::trusted(|| plat::file_exists(&tmp)) == 2 {
                self.m.filedlg.set_dir(&tmp);
                self.fd_load();
                return;
            }
            // a file (or a new name): folder part + name part
            match tmp.rfind('/') {
                Some(k) => {
                    self.m.filedlg.name = copy_cap(&tmp[k + 1..], FD_NAME_CAP);
                    // cut after the separator for roots ("/", "C:/"), at it otherwise
                    if k == 0 || (k == 2 && tmp.as_bytes()[1] == b':') {
                        tmp.truncate(k + 1);
                    } else {
                        tmp.truncate(k);
                    }
                    if super::perm::trusted(|| plat::file_exists(&tmp)) == 2 {
                        self.m.filedlg.set_dir(&tmp);
                        self.fd_load();
                        return;
                    }
                }
                None => self.m.filedlg.name = copy_cap(&tmp, FD_NAME_CAP),
            }
        }
        if !self.m.filedlg.last.is_empty() {
            let last = self.m.filedlg.last.clone();
            self.m.filedlg.set_dir(&last);
            self.fd_load();
            return;
        }
        self.fd_go_home();
    }

    // fd_go_home: switch to the home folder and list it
    fn fd_go_home(&mut self) {
        let h = copy_cap(&plat::home().unwrap_or_else(|| String::from("/")), FD_PATH);
        self.m.filedlg.set_dir(&h);
        self.fd_load();
    }

    // fd_toolbar_row(id): up, home, drives, current folder
    fn fd_toolbar_row(&mut self, id: u32) {
        self.box_begin(0, BOX_H | BOX_XCENTER, 4);
        if is_root(&self.m.filedlg.dir) {
            self.next_flags(ST_DISABLED);
        }
        if self.icon_button(id.wrapping_add(1), IC_UP, tr("Übergeordneter Ordner")) & EV_ACT != 0 {
            self.m.filedlg.parent();
            self.fd_load();
        }
        if self.icon_button(id.wrapping_add(2), IC_HOME, tr("Startordner")) & EV_ACT != 0 {
            self.fd_go_home();
        }
        // drives (at most 8, as many as a list of FD_DRIVES bytes holds)
        let mut used = 0;
        for (k, d) in plat::drives().iter().enumerate().take(8) {
            used += d.len() + 1;
            if used >= FD_DRIVES {
                break;
            }
            // label without the trailing separator of "C:/"
            let mut label = copy_cap(d, 16);
            if label.len() == 3 && label.as_bytes()[1] == b':' {
                label.truncate(2);
            }
            if self.button(id.wrapping_add(3 + k as u32), &label) & EV_ACT != 0 {
                self.m.filedlg.set_dir(d);
                self.fd_load();
            }
        }
        self.spacer(8);
        // the folder, with the native separator
        let disp = copy_cap(&self.m.filedlg.dir, FD_PATH);
        #[cfg(windows)]
        let disp = disp.replace('/', "\\");
        self.next_size(-1, 0);
        self.text(TS_BODY | TX_ELLIPSIS | TX_SECONDARY, &disp);
        self.box_end();
    }

    // fd_row(i): one list row: folder / file icon and the name
    fn fd_row(&mut self, i: i32) {
        self.list_item_begin(i);
        let (name, dir) = match self.m.filedlg.entry(i) {
            Some((n, d)) => (String::from(n), d),
            None => (String::new(), false),
        };
        let marked = self.m.filedlg.marks.len() > 1 && self.m.filedlg.marks.contains(&i);
        self.icon(if dir { IC_FOLDER } else if marked { IC_CHECK_MARK } else { IC_PAGE }, 16);
        self.next_size(-1, 0);
        self.text(TS_BODY | TX_ELLIPSIS, &name);
        self.list_item_end();
    }

    // fd_activate(i) -> entry i is a file (folders are opened here)
    fn fd_activate(&mut self, i: i32) -> bool {
        let f = &mut self.m.filedlg;
        let Some((name, dir)) = f.entry(i) else { return false };
        let name = String::from(name);
        if dir {
            f.enter(&name);
            self.fd_load();
            return false;
        }
        f.name = copy_cap(&name, FD_NAME_CAP);
        true
    }

    // fd_confirm(flags, path, cap) -> the chosen path was copied to path
    fn fd_confirm(&mut self, flags: u32, path: &mut String, cap: u32) -> bool {
        let f = &mut self.m.filedlg;
        if flags & FD_FOLDER != 0 {
            // the selected folder, else the one shown
            let mut d = copy_cap(&f.dir, FD_PATH);
            if let Some((n, true)) = f.entry(f.sel) {
                let n = String::from(n);
                join(&mut d, &n);
            }
            *path = copy_cap(&d, cap as usize);
            f.last = copy_cap(&f.dir, FD_PATH);
            return true;
        }
        if flags & (FD_MULTI | FD_SAVE) == FD_MULTI && f.name.contains('"') {
            return self.fd_confirm_multi(path, cap);
        }
        if f.name.is_empty() {
            // no name: open the selected folder
            if let Some((n, true)) = f.entry(f.sel) {
                let n = String::from(n);
                f.enter(&n);
                self.fd_load();
            }
            return false;
        }
        // absolute name ("/x", "C:/x", "C:\x") or relative to the folder
        let mut tmp = if is_abs(&f.name) {
            copy_cap(&f.name, FD_PATH)
        } else {
            let mut t = copy_cap(&f.dir, FD_PATH);
            join(&mut t, &f.name.clone());
            t
        };
        // a path that filled the buffer was cut: refuse it
        if tmp.len() >= FD_PATH - 16 {
            f.msg = Some(tr("Der Pfad ist zu lang."));
            return false;
        }
        tmp = canon(&norm(&tmp));
        let ex = super::perm::trusted(|| plat::file_exists(&tmp));
        if ex == 2 {
            // a folder name: go there
            self.m.filedlg.set_dir(&tmp);
            self.fd_load();
            self.m.filedlg.name.clear();
            return false;
        }
        let f = &mut self.m.filedlg;
        if flags & FD_SAVE == 0 {
            if ex != 1 {
                f.msg = Some(tr("Die Datei wurde nicht gefunden."));
                return false;
            }
        } else if !f.exts.is_empty() && !f.exts[0].is_empty() {
            // append the first extension when the name has none
            let base = match tmp.rfind('/') {
                Some(k) => &tmp[k..],
                None => &tmp[..],
            };
            if !base.contains('.') {
                cat_cap(&mut tmp, ".", FD_PATH);
                // the selected extension (flags bits 8..15), without its "=description"
                let k = ((flags >> FD_EXT_SHIFT) & 0xFF) as usize;
                let e = f.exts.get(k).filter(|e| !e.is_empty()).unwrap_or(&f.exts[0]);
                let e = e.split(['=', ';']).next().unwrap_or("");
                if tmp.len() + e.len() >= FD_PATH - 1 {
                    f.msg = Some(tr("Der Pfad ist zu lang.")); // the extension does not fit
                    return false;
                }
                tmp.push_str(e);
            }
        }
        *path = copy_cap(&tmp, cap as usize);
        f.paths = alloc::vec![path.clone()];
        f.last = copy_cap(&f.dir, FD_PATH);
        true
    }

    // (E8) FD_MULTI: the quoted names of the name box ("a" "b") -> every one an existing file
    fn fd_confirm_multi(&mut self, path: &mut String, cap: u32) -> bool {
        let f = &mut self.m.filedlg;
        let names: Vec<String> = f.name.split('"').map(|s| s.trim()).filter(|s| !s.is_empty()).map(String::from).collect();
        let mut all = Vec::new();
        for n in &names {
            let mut t = if is_abs(n) {
                copy_cap(n, FD_PATH)
            } else {
                let mut t = copy_cap(&f.dir, FD_PATH);
                join(&mut t, n);
                t
            };
            if t.len() >= FD_PATH - 16 {
                f.msg = Some(tr("Der Pfad ist zu lang."));
                return false;
            }
            t = canon(&norm(&t));
            if super::perm::trusted(|| plat::file_exists(&t)) != 1 {
                f.msg = Some(tr("Die Datei wurde nicht gefunden."));
                return false;
            }
            all.push(copy_cap(&t, cap as usize));
        }
        if all.is_empty() {
            return false;
        }
        *path = all[0].clone();
        f.paths = all;
        f.last = copy_cap(&f.dir, FD_PATH);
        true
    }

    // fd_load: list the folder (folders first, then files matching the extensions)
    fn fd_load(&mut self) {
        let f = &mut self.m.filedlg;
        f.idx.clear();
        f.store.clear();
        f.marks.clear();
        f.anchor = -1;
        f.sel = -1;
        f.msg = None;
        let dir = f.dir.clone();
        let mut ents: Vec<(String, bool)> = Vec::new();
        super::perm::trusted(|| plat::dir_list(&dir, &mut |name: &str, is_dir: bool, _size: u32| ents.push((String::from(name), is_dir))));
        let f = &mut self.m.filedlg;
        for (name, is_dir) in ents {
            // hidden entries, "." and ".."; choosing a folder: folders only
            if name.starts_with('.') || (!is_dir && (f.folder || !f.ext_ok(&name))) {
                continue;
            }
            if f.idx.len() >= FD_MAX || f.store.len() + name.len() + 1 > FD_STORE {
                continue;
            }
            let off = f.store.len() as u32;
            f.store.extend_from_slice(name.as_bytes());
            f.store.push(0);
            f.idx.push(if is_dir { off | FD_DIRBIT } else { off });
        }
        f.sort();
        self.repass();
    }
}
