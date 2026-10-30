// apply -- the state of an update in cfg.dir, putting a staged update in place, the trial of
// the new version and the way back
//
//   state(cfg) -> State          the file cfg.dir/state (key=value lines)
//   startup(cfg) -> Startup      (lib.rs) first thing at program start
//   healthy(cfg) -> bool         (lib.rs) the health marker
//
// Every file of an update is written next to its target as NAME.jfupd-new; the old file is
// copied to backup/files/ (backup/list: "R path" replaced, "N path" new), renamed away
// (NAME.jfupd-old-<n>: Windows can rename a running exe, not replace it) and the new file takes
// its name.  Leftover .jfupd-* files are removed at a later start, once no process uses them.
// An AppImage update has one file, the .AppImage itself ($APPIMAGE): copied (and hashed on the
// way) from staged/ as NAME.jfupd-new, checked as an AppImage whose signature follows the
// running one's (appimage.rs), renamed over the old file (atomic on Linux); the old file is
// the backup.  Large files are copied in pieces, never read whole.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

use plat::pkg::Packaged;

use crate::manifest::{self, Kind, Manifest, Package};
use crate::{install, Config, Error};

const CHUNK: usize = 1 << 16;

/// where an update stands
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// nothing pending
    #[default]
    None,
    /// downloaded and checked: put in place at the next start (or by Updater::apply)
    Staged,
    /// the new version runs on trial until healthy()
    Trial,
    /// the new version reported healthy() (the backup of the old one is kept)
    Ok,
    /// the new version failed and the old one is back; reported once by startup()
    RolledBack,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::None => "",
            Stage::Staged => "staged",
            Stage::Trial => "trial",
            Stage::Ok => "ok",
            Stage::RolledBack => "rolledback",
        }
    }

    fn parse(s: &str) -> Stage {
        match s {
            "staged" => Stage::Staged,
            "trial" => Stage::Trial,
            "ok" => Stage::Ok,
            "rolledback" => Stage::RolledBack,
            _ => Stage::None,
        }
    }
}

/// the file cfg.dir/state
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub stage: Stage,
    /// the staged, new or rolled-back version
    pub version: String,
    /// the version that was running before it
    pub from: String,
    /// starts of the new version without healthy()
    pub starts: u32,
    /// what the staged file is
    pub kind: String,
    /// the staged file's name in staged/
    pub file: String,
    /// versions that were rolled back: never offered again
    pub bad: Vec<String>,
    /// the newest version a signed manifest named (older manifests are refused: replay)
    pub seen: String,
}

impl State {
    fn text(&self) -> String {
        format!(
            "stage={}\nversion={}\nfrom={}\nstarts={}\nkind={}\nfile={}\nbad={}\nseen={}\n",
            self.stage.name(),
            self.version,
            self.from,
            self.starts,
            self.kind,
            self.file,
            self.bad.join(" "),
            self.seen
        )
    }

    fn parse(text: &str) -> State {
        let mut s = State::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "stage" => s.stage = Stage::parse(v),
                "version" => s.version = String::from(v),
                "from" => s.from = String::from(v),
                "starts" => s.starts = v.parse().unwrap_or(0),
                "kind" => s.kind = String::from(v),
                "file" => s.file = if rel_ok(v) && !v.contains('/') { String::from(v) } else { String::new() },
                "bad" => s.bad = v.split_whitespace().map(String::from).collect(),
                "seen" => s.seen = String::from(v),
                _ => {}
            }
        }
        s
    }
}

/// a path in cfg.dir
pub(crate) fn path(cfg: &Config, name: &str) -> String {
    format!("{}/{}", cfg.dir.trim_end_matches(['/', '\\']), name)
}

/// the state of cfg.dir (a missing or unreadable file: nothing pending)
pub fn state(cfg: &Config) -> State {
    plat::read_file(&path(cfg, "state")).map(|b| State::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
}

pub(crate) fn save(cfg: &Config, s: &State) -> Result<(), Error> {
    if plat::mkdir(&cfg.dir) && plat::write_file(&path(cfg, "state"), s.text().as_bytes()) {
        Ok(())
    } else {
        Err(Error::Io(format!("{} kann nicht geschrieben werden", path(cfg, "state"))))
    }
}

/// the lock of cfg.dir while a download, an apply or a state change runs (None: another
/// process holds it)
pub(crate) fn lock(cfg: &Config) -> Option<plat::FileLock> {
    if !plat::mkdir(&cfg.dir) {
        return None;
    }
    plat::FileLock::acquire(&path(cfg, "lock"))
}

/// delete a file or a folder with everything in it
pub(crate) fn remove_tree(p: &str) {
    let mut names = Vec::new();
    plat::dir_list(p, &mut |n, dir, _| names.push((String::from(n), dir)));
    for (n, dir) in names {
        let c = format!("{}/{}", p, n);
        if dir {
            remove_tree(&c);
        } else {
            plat::delete(&c);
        }
    }
    plat::delete(p);
}

/// the name of a way of installing that updates itself as a whole (not by exe / bundle files)
pub(crate) fn sealed_name(p: &Packaged) -> &'static str {
    match p {
        Packaged::Plain => "",
        Packaged::AppImage(_) => "AppImage",
        Packaged::Flatpak(_) => "Flatpak",
        Packaged::Msix(_) => "MSIX-Paket",
        Packaged::Snap => "Snap",
    }
}

/// the program file: cfg.exe, the running AppImage's file, or the running exe (Linux: without
/// " (deleted)")
pub(crate) fn program(cfg: &Config) -> Option<String> {
    if !cfg.exe.is_empty() {
        return Some(cfg.exe.clone());
    }
    if let Packaged::AppImage(p) = install(cfg) {
        return Some(p);
    }
    let p = plat::current_exe()?;
    Some(String::from(p.strip_suffix(" (deleted)").unwrap_or(&p)))
}

// the rule of PERMISSIONS that lets an update write the program's folder
fn rule(cfg: &Config) -> &'static str {
    if matches!(install(cfg), Packaged::AppImage(_)) { "fs.write $APPIMAGE/**" } else { "fs.write $EXE/**" }
}

/// whether the AppImage file can be replaced: its folder takes a new file -> Error::ReadOnly
/// (the folder cannot be written: not worked around) or Error::Io (refused by PERMISSIONS)
pub(crate) fn replaceable(cfg: &Config, target: &str) -> Result<(), Error> {
    let (dir, name) = split_dir(target);
    if !plat::policy::fs_allowed(target, plat::policy::FS_WRITE) {
        return Err(not_writable(cfg, target));
    }
    let probe = aside(&format!("{}/{}", dir, name), "probe");
    match plat::File::create_new(&probe) {
        Some(f) => {
            drop(f);
            plat::delete(&probe);
            Ok(())
        }
        None => Err(Error::ReadOnly(String::from(dir))),
    }
}

fn split_dir(p: &str) -> (&str, &str) {
    match p.rfind(['/', '\\']) {
        Some(i) => (&p[..i], &p[i + 1..]),
        None => (".", p),
    }
}

// a file written, its folder created first (plat::write_file creates folders only in mounts)
fn write(p: &str, data: &[u8]) -> bool {
    let (dir, _) = split_dir(p);
    (plat::file_exists(dir) == 2 || plat::mkdir(dir)) && plat::write_file(p, data)
}

// a path inside a bundle that may become a file of the program's folder
fn rel_ok(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains(['\\', ':', '\0'])
        && !rel.contains(".jfupd-")
        && rel.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

/// the staged manifest, checked again: signature, app, version, expiry, replay -> it and the
/// package of this system and installation
fn staged_manifest(cfg: &Config, st: &State) -> Result<(Manifest, Package), Error> {
    let read = |n: &str| plat::read_file(&path(cfg, &format!("staged/{}", n)));
    let (Some(m), Some(sig)) = (read("update.json"), read("update.json.sig")) else {
        return Err(Error::Io(String::from("das bereitgestellte Update fehlt")));
    };
    if !manifest::verify(&m, &sig, &cfg.keys) {
        return Err(Error::Signature);
    }
    let man = Manifest::parse(&m).map_err(Error::Manifest)?;
    let pkg = man.package_for(&cfg.target, Kind::parse(&st.kind) == Some(Kind::AppImage)).cloned().ok_or(Error::Nothing)?;
    if man.expires != 0 && crate::now_secs() > man.expires {
        return Err(Error::Expired);
    }
    if !st.seen.is_empty() && manifest::newer(&st.seen, &man.version) {
        return Err(Error::Replay(String::from(&st.seen)));
    }
    if !manifest::newer(&man.version, &cfg.version) {
        return Err(Error::Manifest(String::from("das bereitgestellte Update ist nicht neuer als die laufende Version")));
    }
    if man.app != cfg.app || man.version != st.version || pkg.kind.name() != st.kind {
        return Err(Error::Manifest(String::from("das bereitgestellte Update passt nicht zum Zustand")));
    }
    Ok((man, pkg))
}

/// the size and SHA-256 of a file, read in pieces (None: unreadable)
fn file_sha256(p: &str) -> Option<(u64, [u8; 32])> {
    let f = plat::File::open(p, plat::FO_READ)?;
    let mut h = crypto::Sha256::new();
    let mut buf = alloc::vec![0u8; CHUNK];
    let mut off = 0u64;
    loop {
        let n = f.read_at(off, &mut buf)?;
        if n == 0 {
            return Some((off, h.finalize()));
        }
        h.update(&buf[..n]);
        off += n as u64;
    }
}

/// the staged update, checked again: the manifest (staged_manifest), the file's size and
/// SHA-256 (read in pieces) -> the manifest, the package and the file's path
pub(crate) fn staged_ok(cfg: &Config, st: &State) -> Result<(Manifest, Package, String), Error> {
    let (man, pkg) = staged_manifest(cfg, st)?;
    let file = path(cfg, &format!("staged/{}", st.file));
    let (size, hash) = file_sha256(&file).ok_or_else(|| Error::Io(String::from("die bereitgestellte Datei fehlt")))?;
    if size != pkg.size {
        return Err(Error::Size { expected: pkg.size, got: size });
    }
    if !crypto::ct_eq(&hash, &pkg.sha256) {
        return Err(Error::Hash);
    }
    Ok((man, pkg, file))
}

/// the staged update's bytes (an exe or a bundle: small), checked again as staged_ok does, on
/// the very bytes that are put in place
fn staged_bytes(cfg: &Config, st: &State) -> Result<(Package, Vec<u8>), Error> {
    let (_, pkg) = staged_manifest(cfg, st)?;
    let data = plat::read_file(&path(cfg, &format!("staged/{}", st.file))).ok_or_else(|| Error::Io(String::from("die bereitgestellte Datei fehlt")))?;
    if data.len() as u64 != pkg.size {
        return Err(Error::Size { expected: pkg.size, got: data.len() as u64 });
    }
    if !crypto::ct_eq(&crypto::sha256(&data), &pkg.sha256) {
        return Err(Error::Hash);
    }
    Ok((pkg, data))
}

// where the bytes of a new file come from
enum Src {
    Bytes(Vec<u8>),
    /// a file, copied in pieces; with the SHA-256 it must have (checked while copying)
    File(String, Option<[u8; 32]>),
}

// the files an update writes into the program's folder: (relative path, bytes)
fn update_files(kind: Kind, exe_name: &str, data: Vec<u8>) -> Result<Vec<(String, Vec<u8>)>, Error> {
    match kind {
        Kind::Exe => Ok(alloc::vec![(String::from(exe_name), data)]),
        Kind::Bundle => {
            let b = plat::vfs::Bundle::new(data).ok_or_else(|| Error::Manifest(String::from("das Bündel ist beschädigt")))?;
            let mut out = Vec::new();
            for n in b.names() {
                if !rel_ok(n) {
                    return Err(Error::Manifest(format!("unzulässiger Pfad im Bündel: {}", n)));
                }
                let bytes = plat::vfs::Provider::read(&b, n).ok_or_else(|| Error::Manifest(format!("{} im Bündel ist beschädigt", n)))?;
                out.push((String::from(n), bytes));
            }
            if out.is_empty() {
                return Err(Error::Manifest(String::from("das Bündel ist leer")));
            }
            Ok(out)
        }
        _ => Err(Error::Unsupported(String::from("ein Installationspaket wird nicht von selbst eingespielt (Updater::staged)"))),
    }
}

static SEQ: AtomicU32 = AtomicU32::new(0);

// a fresh name next to target for the file that leaves its place
fn aside(target: &str, what: &str) -> String {
    format!("{}.jfupd-{}-{:x}{:x}", target, what, plat::time_us(), SEQ.fetch_add(1, SeqCst))
}

fn not_writable(cfg: &Config, p: &str) -> Error {
    Error::Io(format!("{} kann nicht geschrieben werden (Rechte des Programmordners; in PERMISSIONS: {})", p, rule(cfg)))
}

// copy the file src into f (from offset 0) in pieces -> its SHA-256 (None: a read or a write
// failed)
fn copy_into(src: &str, f: &plat::File) -> Option<[u8; 32]> {
    let from = plat::File::open(src, plat::FO_READ)?;
    let mut h = crypto::Sha256::new();
    let mut buf = alloc::vec![0u8; CHUNK];
    let mut off = 0u64;
    loop {
        let n = from.read_at(off, &mut buf)?;
        if n == 0 {
            return Some(h.finalize());
        }
        h.update(&buf[..n]);
        if !crate::fetch::write_all(f, off, &buf[..n]) {
            return None;
        }
        off += n as u64;
    }
}

// a copy of src at dst (its folder made first) -> whether it worked
fn copy_file(src: &str, dst: &str) -> bool {
    let (dir, _) = split_dir(dst);
    if plat::file_exists(dir) != 2 && !plat::mkdir(dir) {
        return false;
    }
    plat::delete(dst);
    let Some(f) = plat::File::open(dst, plat::FO_WRITE) else { return false };
    let ok = copy_into(src, &f).is_some() && f.sync();
    drop(f);
    if !ok {
        plat::delete(dst);
    }
    ok
}

// write src as target: NAME.jfupd-new first, then the old file renamed away and the new one
// renamed into its place; an AppImage (appimage = true) is checked as the follower of the
// running one before it takes the place
fn swap(cfg: &Config, target: &str, src: &Src, appimage: bool) -> Result<(), Error> {
    let new = format!("{}.jfupd-new", target);
    let (dir, _) = split_dir(target);
    if plat::file_exists(dir) != 2 && !plat::mkdir(dir) {
        return Err(not_writable(cfg, target));
    }
    // never write through a name that already exists (a leftover or a planted link)
    plat::delete(&new);
    #[cfg(any(windows, target_os = "linux"))]
    let file = plat::File::create_new(&new);
    #[cfg(not(any(windows, target_os = "linux")))]
    let file = plat::File::open(&new, plat::FO_RW);
    let mut wrong: Option<Error> = None;
    let written = file.is_some_and(|f| {
        // the old file's permissions (a new file: executable, like a program), before the data
        #[cfg(target_os = "linux")]
        {
            let mode = plat::fs::stat(target).map_or(0o755, |m| m.mode & 0o7777);
            plat::fs::set_mode(&new, mode);
        }
        let ok = match src {
            Src::Bytes(data) => crate::fetch::write_all(&f, 0, data),
            Src::File(p, want) => match copy_into(p, &f) {
                Some(h) if want.is_some_and(|w| !crypto::ct_eq(&h, &w)) => {
                    wrong = Some(Error::Hash);
                    false
                }
                Some(_) => true,
                None => false,
            },
        };
        ok && f.sync()
    });
    if let Some(e) = wrong {
        plat::delete(&new);
        return Err(e);
    }
    if !written {
        plat::delete(&new);
        return Err(not_writable(cfg, target));
    }
    if appimage {
        if let Err(why) = crate::appimage::follows(target, &new) {
            plat::delete(&new);
            return Err(Error::Rejected(why));
        }
    }
    let exists = plat::file_exists(target) == 1;
    if cfg!(windows) && exists {
        let old = aside(target, "old");
        if !plat::rename(target, &old) {
            plat::delete(&new);
            return Err(not_writable(cfg, target));
        }
        if !plat::rename(&new, target) {
            plat::rename(&old, target);
            plat::delete(&new);
            return Err(not_writable(cfg, target));
        }
        plat::fs::sync_dir(dir);
        return Ok(());
    }
    if !plat::rename(&new, target) {
        plat::delete(&new);
        return Err(not_writable(cfg, target));
    }
    plat::fs::sync_dir(dir);
    Ok(())
}

// remove a file of the program's folder that may be in use (Windows: renamed away first)
fn retire(target: &str) {
    if plat::delete(target) {
        return;
    }
    let bad = aside(target, "bad");
    plat::rename(target, &bad);
}

/// put the staged update in place (the lock is held) -> Ok: stage Trial
pub(crate) fn apply_staged(cfg: &Config, st: &mut State) -> Result<(), Error> {
    let inst = install(cfg);
    let kind = Kind::parse(&st.kind);
    // dropped when it can never apply here: it would be tried (hashed, backed up) at every start
    let refuse = |st: &mut State, why: String| {
        drop_staged(cfg, st);
        Err(Error::Unsupported(why))
    };
    match (kind, &inst) {
        (Some(Kind::Package), _) => {
            return Err(Error::Unsupported(String::from("ein Installationspaket wird nicht von selbst eingespielt (Updater::staged)")));
        }
        (Some(Kind::AppImage), Packaged::AppImage(_)) | (Some(Kind::Exe | Kind::Bundle), Packaged::Plain) => {}
        (Some(Kind::AppImage), _) => return refuse(st, String::from("ein AppImage-Update, aber das Programm läuft nicht als AppImage")),
        (Some(Kind::Exe | Kind::Bundle), other) => {
            return refuse(st, format!("das Programm läuft als {}; es wird über dieses Paket aktualisiert", sealed_name(other)));
        }
        _ => return refuse(st, format!("unbekannte Art „{}“", st.kind)),
    }
    let appimage = kind == Some(Kind::AppImage);
    let exe = program(cfg).ok_or_else(|| Error::Io(String::from("der Programmpfad ist unbekannt")))?;
    let (dir, exe_name) = split_dir(&exe);
    let files: Vec<(String, Src)> = if appimage {
        match staged_ok(cfg, st) {
            Ok((_, pkg, file)) => alloc::vec![(String::from(exe_name), Src::File(file, Some(pkg.sha256)))],
            Err(e) => {
                // a damaged or foreign staged update is dropped, so it does not block every start
                drop_staged(cfg, st);
                return Err(e);
            }
        }
    } else {
        let made = staged_bytes(cfg, st).and_then(|(pkg, data)| update_files(pkg.kind, exe_name, data));
        match made {
            Ok(f) => f.into_iter().map(|(r, b)| (r, Src::Bytes(b))).collect(),
            Err(e) => {
                drop_staged(cfg, st);
                return Err(e);
            }
        }
    };
    // the backup of every file the update touches
    let backup = path(cfg, "backup");
    let fresh = path(cfg, "backup.new");
    remove_tree(&fresh);
    let mut list = format!("from={}\n", cfg.version);
    for (rel, _) in &files {
        let target = format!("{}/{}", dir, rel);
        if plat::file_exists(&target) == 1 {
            if !copy_file(&target, &format!("{}/files/{}", fresh, rel)) {
                return Err(Error::Io(format!("die Sicherung von {} in {} kann nicht geschrieben werden", target, backup)));
            }
            list.push_str(&format!("R {}\n", rel));
        } else {
            list.push_str(&format!("N {}\n", rel));
        }
    }
    if !write(&format!("{}/list", fresh), list.as_bytes()) {
        return Err(Error::Io(format!("die Sicherung in {} kann nicht geschrieben werden", backup)));
    }
    // the previous backup goes only now that the new one is complete
    remove_tree(&backup);
    if !plat::rename(&fresh, &backup) {
        return Err(Error::Io(format!("die Sicherung in {} kann nicht geschrieben werden", backup)));
    }
    // the new files; on a failure the old ones come back
    for (i, (rel, src)) in files.iter().enumerate() {
        if let Err(e) = swap(cfg, &format!("{}/{}", dir, rel), src, appimage) {
            restore(cfg, dir, Some(&files[..i].iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>()));
            if matches!(e, Error::Rejected(_) | Error::Hash) {
                drop_staged(cfg, st); // it would be refused at every start
            }
            return Err(e);
        }
    }
    st.stage = Stage::Trial;
    st.from = String::from(&cfg.version);
    st.starts = 0;
    save(cfg, st)
}

// the backed-up state of the files (only: of these; new ones removed)
fn restore(cfg: &Config, dir: &str, only: Option<&[&str]>) -> bool {
    let backup = path(cfg, "backup");
    let Some(list) = plat::read_file(&format!("{}/list", backup)) else { return false };
    let mut ok = true;
    for line in String::from_utf8_lossy(&list).lines() {
        let Some((op, rel)) = line.split_once(' ') else { continue };
        if !rel_ok(rel) || only.is_some_and(|o| !o.contains(&rel)) {
            continue;
        }
        let target = format!("{}/{}", dir, rel);
        match op {
            "R" => {
                let old = format!("{}/files/{}", backup, rel);
                ok &= plat::file_exists(&old) == 1 && swap(cfg, &target, &Src::File(old, None), false).is_ok();
            }
            "N" => retire(&target),
            _ => {}
        }
    }
    ok
}

/// the old version back (the lock is held) -> Ok: stage RolledBack, the version recorded as bad
pub(crate) fn rollback(cfg: &Config, st: &mut State) -> Result<(), Error> {
    let exe = program(cfg).ok_or_else(|| Error::Io(String::from("der Programmpfad ist unbekannt")))?;
    let (dir, _) = split_dir(&exe);
    if plat::file_exists(&path(cfg, "backup/list")) != 1 {
        return Err(Error::Io(String::from("keine Sicherung der alten Version")));
    }
    if !restore(cfg, dir, None) {
        return Err(Error::Io(format!("die alte Version kann nicht nach {} zurückgeschrieben werden", dir)));
    }
    if !st.bad.contains(&st.version) {
        st.bad.push(st.version.clone());
    }
    st.version = core::mem::take(&mut st.from);
    st.stage = Stage::RolledBack;
    st.starts = 0;
    remove_tree(&path(cfg, "staged"));
    save(cfg, st)
}

/// forget the staged update (its files and the stage)
pub(crate) fn drop_staged(cfg: &Config, st: &mut State) {
    remove_tree(&path(cfg, "staged"));
    st.stage = Stage::None;
    st.version.clear();
    st.kind.clear();
    st.file.clear();
    let _ = save(cfg, st);
}

/// remove what earlier updates left in the program's folder (NAME.jfupd-*: files of ended
/// processes; a file still in use stays for a later start)
pub(crate) fn cleanup(cfg: &Config) {
    let Some(exe) = program(cfg) else { return };
    let (dir, _) = split_dir(&exe);
    let mut dirs = alloc::vec![String::from(dir)];
    if let Some(list) = plat::read_file(&path(cfg, "backup/list")) {
        for line in String::from_utf8_lossy(&list).lines() {
            if let Some((_, rel)) = line.split_once(' ').filter(|(_, r)| rel_ok(r) && r.contains('/')) {
                let d = format!("{}/{}", dir, split_dir(rel).0);
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
    }
    // the folder of an AppImage holds other files: only those of its own name
    let own = matches!(install(cfg), Packaged::AppImage(_)).then(|| format!("{}.jfupd-", split_dir(&exe).1));
    for d in dirs {
        let mut old = Vec::new();
        plat::dir_list(&d, &mut |n, is_dir, _| {
            if !is_dir && own.as_deref().is_none_or(|o| n.starts_with(o))
                && (n.contains(".jfupd-old-") || n.contains(".jfupd-bad-") || n.contains(".jfupd-probe-") || n.ends_with(".jfupd-new"))
            {
                old.push(n.to_string());
            }
        });
        for n in old {
            plat::delete(&format!("{}/{}", d, n));
        }
    }
}
