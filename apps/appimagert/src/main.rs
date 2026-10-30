// appimagert -- the AppImage runtime of JFLib (python tools/package.py APP --formats appimage):
// a static Linux program without libc in front of a SquashFS image of the AppDir.  The
// AppImage type 2 layout: "AI\x02" at byte 8 of the ELF header and the update information in
// a section .upd_info (both added to the built file by tools/pkg_linux.py), the image starts
// where this ELF file ends (its section header table, docs/PACKAGING.md "AppImage").
//
// It does not mount the image (no FUSE needed): the first start extracts it into
// $XDG_CACHE_HOME/appimage-run/<name>-<xxh64 of the image> (into a part folder that is
// renamed into place, so parallel starts and broken runs never leave a half folder in use);
// later starts of the same file reuse it, and folders of other versions of the same name
// that were not used for 7 days are removed.  Then AppRun of that folder (a relative link:
// its target) replaces this process (plat::process::exec) with APPIMAGE (this file), APPDIR,
// ARGV0 and OWD set, as the upstream runtime does.
//
//   NAME.AppImage ARGS..                 run the app
//   --appimage-extract [PATTERN]         extract into ./squashfs-root (PATTERN: * and ? on the
//                                        path inside the image)
//   --appimage-extract-and-run ARGS..    the same as without it (this runtime always extracts)
//   --appimage-offset                    where the image starts in the file
//   --appimage-updateinformation         the update information (.upd_info)
//   --appimage-signature                 the embedded GPG signature (.sha256_sig, written by
//                                        tools/pkg_linux.py as appimagetool --sign does)
//   --appimage-portable-home / -config   make NAME.AppImage.home / .config next to the file:
//                                        HOME / XDG_CONFIG_HOME of the app while it exists
//   --appimage-version, --appimage-help
//   --appimage-mount                     refused: this runtime does not mount (no FUSE)
//
// TARGET_APPIMAGE=FILE uses another file's image (tests, a runtime without an image).  Exit
// code: the app's; 127 when AppRun does not start; 1 for errors of the runtime (on stderr).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod sqfs;

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use plat::fs::{join, lstat, read_dir};
use plat::{FO_READ, FO_WRITE, File};
use sqfs::{Image, Inode, Kind};

const VERSION: &str = "1 (JFLib, extract-and-run)";
const KEEP_DAYS: i64 = 7;
const MAX_DEPTH: usize = 128;

rt::main!(run());

fn fail(msg: &str) -> i32 {
    plat::write_stderr(format!("appimagert: {msg}\n").as_bytes());
    1
}

fn run() -> i32 {
    let args = plat::args();
    let argv0 = args.first().cloned().unwrap_or_default();
    let me = match plat::env("TARGET_APPIMAGE").filter(|p| !p.is_empty()) {
        Some(p) if p.starts_with('/') => p,
        Some(p) => join(&plat::cwd(), &p),
        None => match plat::current_exe() {
            Some(p) => p,
            None => return fail("der eigene Pfad ist unbekannt (/proc/self/exe)"),
        },
    };
    let Some(file) = File::open(&me, FO_READ) else { return fail(&format!("{me} lässt sich nicht lesen")) };
    let Some((offset, sections)) = elf_layout(&file) else { return fail(&format!("{me} ist keine ELF-Datei")) };
    let opt = args.get(1).map(String::as_str).unwrap_or("");
    let rest = |from: usize| -> Vec<&str> { args.iter().skip(from).map(String::as_str).collect() };
    match opt {
        "--appimage-help" => {
            plat::write_stdout(HELP.as_bytes());
            0
        }
        "--appimage-version" => {
            plat::write_stdout(format!("AppImage-Runtime {VERSION}\n").as_bytes());
            0
        }
        "--appimage-offset" => {
            plat::write_stdout(format!("{offset}\n").as_bytes());
            0
        }
        "--appimage-updateinformation" | "--appimage-updateinfo" | "--appimage-signature" => {
            let name = if opt == "--appimage-signature" { ".sha256_sig" } else { ".upd_info" };
            let info = section(&file, &sections, name).unwrap_or_default();
            let n = info.iter().position(|&b| b == 0).unwrap_or(info.len());
            plat::write_stdout(&info[..n]);
            plat::write_stdout(b"\n");
            0
        }
        "--appimage-portable-home" | "--appimage-portable-config" => {
            let dir = format!("{me}.{}", if opt.ends_with("home") { "home" } else { "config" });
            if !plat::mkdir(&dir) {
                return fail(&format!("{dir} wurde nicht angelegt"));
            }
            plat::write_stdout(format!("Angelegt: {dir}\n").as_bytes());
            0
        }
        "--appimage-mount" => fail("diese Runtime hängt das Abbild nicht ein (kein FUSE); --appimage-extract entpackt es"),
        "--appimage-extract" => {
            let mut img = match Image::open(&file, offset) {
                Ok(i) => i,
                Err(e) => return fail(&e),
            };
            let dest = join(&plat::cwd(), "squashfs-root");
            match extract(&mut img, &dest, args.get(2).map(String::as_str)) {
                Ok(n) => {
                    plat::write_stdout(format!("{n} Einträge nach {dest} entpackt\n").as_bytes());
                    0
                }
                Err(e) => fail(&e),
            }
        }
        "--appimage-extract-and-run" => launch(&file, offset, &me, &argv0, &rest(2)),
        o if o.starts_with("--appimage-") => fail(&format!("unbekannte Option {o} (--appimage-help)")),
        _ => launch(&file, offset, &me, &argv0, &rest(1)),
    }
}

const HELP: &str = "AppImage-Runtime von JFLib: entpackt das Abbild beim ersten Start nach
$XDG_CACHE_HOME/appimage-run und startet AppRun (kein FUSE nötig).

  --appimage-extract [MUSTER]       nach ./squashfs-root entpacken
  --appimage-extract-and-run ...    wie ein normaler Start
  --appimage-offset                 Beginn des Abbilds in der Datei
  --appimage-updateinformation      Update-Information (.upd_info)
  --appimage-signature              die GPG-Signatur (.sha256_sig; appimagetool --sign)
  --appimage-portable-home          DATEI.home anlegen (HOME des Programms)
  --appimage-portable-config        DATEI.config anlegen (XDG_CONFIG_HOME des Programms)
  --appimage-version, --appimage-help\n";

// ---------------------------------------------------------------- the ELF file in front

/// where the ELF file ends (the image starts there) and its sections (name offset, file
/// offset, size; the index of the section names)
fn elf_layout(f: &File) -> Option<(u64, (Vec<(u32, u64, u64)>, usize))> {
    let mut h = [0u8; 64];
    if !sqfs::read_full(f, 0, &mut h) || &h[..4] != b"\x7fELF" || h[4] != 2 || h[5] != 1 {
        return None;
    }
    let u = |o: usize, n: usize| h[o..o + n].iter().rev().fold(0u64, |a, &x| a << 8 | x as u64);
    let (phoff, shoff) = (u(0x20, 8), u(0x28, 8));
    let (phsize, phnum, shsize, shnum, names) = (u(0x36, 2), u(0x38, 2), u(0x3A, 2), u(0x3C, 2), u(0x3E, 2) as usize);
    let mut end = shoff.checked_add(shsize * shnum)?;
    for i in 0..phnum {
        let mut p = [0u8; 56];
        if phsize < 56 || !sqfs::read_full(f, phoff + i * phsize, &mut p) {
            return None;
        }
        let v = |o: usize| p[o..o + 8].iter().rev().fold(0u64, |a, &x| a << 8 | x as u64);
        end = end.max(v(8).checked_add(v(32))?);
    }
    let mut sections = Vec::new();
    for i in 0..shnum {
        let mut s = [0u8; 64];
        if shsize < 64 || !sqfs::read_full(f, shoff + i * shsize, &mut s) {
            return None;
        }
        let v = |o: usize, n: usize| s[o..o + n].iter().rev().fold(0u64, |a, &x| a << 8 | x as u64);
        let (typ, off, size) = (v(4, 4), v(24, 8), v(32, 8));
        if typ != 8 {
            end = end.max(off.checked_add(size)?); // SHT_NOBITS takes no room in the file
        }
        sections.push((v(0, 4) as u32, off, size));
    }
    Some((end, (sections, names)))
}

/// the bytes of a section by name
fn section(f: &File, (list, names): &(Vec<(u32, u64, u64)>, usize), name: &str) -> Option<Vec<u8>> {
    let &(_, noff, nsize) = list.get(*names)?;
    let mut table = vec![0u8; usize::try_from(nsize).ok().filter(|&n| n <= 1 << 20)?];
    if !sqfs::read_full(f, noff, &mut table) {
        return None;
    }
    let &(_, off, size) = list.iter().find(|(n, _, _)| {
        let s = table.get(*n as usize..).unwrap_or(&[]);
        s.strip_prefix(name.as_bytes()).is_some_and(|r| r.first() == Some(&0))
    })?;
    let mut v = vec![0u8; usize::try_from(size).ok().filter(|&n| n <= 1 << 20)?];
    sqfs::read_full(f, off, &mut v).then_some(v)
}

// ---------------------------------------------------------------- run

fn launch(file: &File, offset: u64, me: &str, argv0: &str, args: &[&str]) -> i32 {
    let mut img = match Image::open(file, offset) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    let Some(key) = image_key(file, offset) else { return fail(&format!("{me} lässt sich nicht lesen")) };
    let Some(base) = plat::cache_dir().or_else(plat::temp_dir) else { return fail("kein Cache-Ordner ($XDG_CACHE_HOME, $HOME)") };
    let root = format!("{base}/appimage-run");
    let stem = stem(me);
    let dir = format!("{root}/{stem}-{key}");
    let ready = lstat(&format!("{dir}/AppRun")).is_some();
    if !ready {
        if lstat(&dir).is_some() {
            remove_tree(&dir); // left incomplete by hand: extract it again
        }
        let part = format!("{dir}.part-{:016x}", plat::random_u64());
        if let Err(e) = extract(&mut img, &part, None) {
            remove_tree(&part);
            return fail(&e);
        }
        if !plat::rename(&part, &dir) {
            remove_tree(&part);
            if lstat(&format!("{dir}/AppRun")).is_none() {
                return fail(&format!("{dir} wurde nicht angelegt"));
            }
        }
        prune(&root, &stem, &dir);
    }
    let now = plat::unix_time_ns();
    plat::fs::set_times(&dir, now, now); // the last use, for prune()
    let apprun = format!("{dir}/AppRun");
    let cwd = plat::cwd();
    let mut env: Vec<(&str, Option<&str>)> = vec![("APPIMAGE", Some(me)), ("APPDIR", Some(&dir)), ("ARGV0", Some(argv0)), ("OWD", Some(&cwd))];
    let (home, config) = (format!("{me}.home"), format!("{me}.config"));
    if plat::file_exists(&home) == 2 {
        env.push(("HOME", Some(&home)));
    }
    if plat::file_exists(&config) == 2 {
        env.push(("XDG_CONFIG_HOME", Some(&config)));
    }
    // AppRun as a relative link: start its target, so argv[0] names the program (JFLib's window
    // class and Wayland app id, StartupWMClass of the .desktop file), not "AppRun"
    let program = match plat::fs::read_link(&apprun) {
        Some(t) if !t.is_empty() && !t.starts_with('/') => format!("{dir}/{t}"),
        _ => apprun,
    };
    let e = plat::process::exec(&program, args, &env);
    fail(&format!("{program} startet nicht (Fehler {e})"));
    127
}

/// xxh64 over the image's xxh64 per MiB and its length: the name of its extracted folder
fn image_key(f: &File, offset: u64) -> Option<String> {
    let end = f.size()?;
    let mut buf = vec![0u8; 1 << 20];
    let mut sums = Vec::new();
    let mut pos = offset;
    while pos < end {
        let n = (end - pos).min(buf.len() as u64) as usize;
        if !sqfs::read_full(f, pos, &mut buf[..n]) {
            return None;
        }
        sums.extend_from_slice(&compress::xxh64(&buf[..n], 0).to_le_bytes());
        pos += n as u64;
    }
    sums.extend_from_slice(&(end - offset).to_le_bytes());
    Some(format!("{:016x}", compress::xxh64(&sums, 0)))
}

/// the file name without ".AppImage", only [A-Za-z0-9._-]
fn stem(path: &str) -> String {
    let name = &path[path.rfind('/').map_or(0, |k| k + 1)..];
    let name = name.strip_suffix(".AppImage").or_else(|| name.strip_suffix(".appimage")).unwrap_or(name);
    let s: String = name.chars().take(64).map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    if s.is_empty() { String::from("app") } else { s }
}

/// remove the folders of other images of the same name not used for KEEP_DAYS (a running
/// older version keeps its folder: each start marks it as used)
fn prune(root: &str, stem: &str, keep: &str) {
    let limit = plat::unix_time_ns() - KEEP_DAYS * 86_400 * 1_000_000_000;
    for e in read_dir(root).unwrap_or_default() {
        let path = format!("{root}/{}", e.name);
        let Some(key) = e.name.strip_prefix(stem).and_then(|r| r.strip_prefix('-')) else { continue };
        let hex = key.as_bytes().get(..16).is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit));
        let other = key.len() == 16 || key.get(16..).is_some_and(|r| r.starts_with(".part-"));
        if path != keep && hex && other && lstat(&path).is_some_and(|m| m.is_dir() && m.mtime_ns < limit) {
            remove_tree(&path);
        }
    }
}

/// a file, a link or a folder with everything in it (links are removed, never followed)
fn remove_tree(path: &str) -> bool {
    if lstat(path).is_some_and(|m| m.is_dir()) {
        for e in read_dir(path).unwrap_or_default() {
            remove_tree(&format!("{path}/{}", e.name));
        }
    }
    plat::delete(path)
}

// ---------------------------------------------------------------- extraction

/// write the image's tree below dest (only the paths matching pattern, with their folders)
/// -> the number of entries written
fn extract(img: &mut Image, dest: &str, pattern: Option<&str>) -> Result<u64, String> {
    let broken = || String::from("beschädigtes SquashFS-Abbild");
    let root = img.root().filter(|r| r.kind == Kind::Dir).ok_or_else(broken)?;
    if !plat::mkdir(dest) {
        return Err(format!("{dest} wurde nicht angelegt"));
    }
    let mut count = 0u64;
    let mut modes: Vec<(String, u16)> = Vec::new(); // folders: set after their contents
    let mut stack: Vec<(Inode, String, usize)> = vec![(root, String::new(), 0)];
    while let Some((dir, rel, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(String::from("zu tief verschachtelte Ordner im Abbild"));
        }
        let entries = img.list(&dir).ok_or_else(broken)?;
        let mut seen = BTreeSet::new();
        for e in &entries {
            if e.name.is_empty() || e.name == "." || e.name == ".." || e.name.contains(['/', '\0']) || !seen.insert(e.name.as_str()) {
                return Err(format!("ungültiger Name im Abbild: {:?}", e.name));
            }
            let path_in = if rel.is_empty() { e.name.clone() } else { format!("{rel}/{}", e.name) };
            let node = img.inode(e.inode).ok_or_else(broken)?;
            let target = format!("{dest}/{path_in}");
            let wanted = pattern.is_none_or(|p| matches(p.as_bytes(), path_in.as_bytes()));
            if wanted && pattern.is_some() {
                // only some entries: their folders were not made on the way down
                let mut at = String::from(dest);
                for part in rel.split('/').filter(|p| !p.is_empty()) {
                    at = format!("{at}/{part}");
                    make_dir(&at)?;
                }
            }
            match node.kind {
                Kind::Dir => {
                    if wanted {
                        make_dir(&target)?;
                        modes.push((target, node.mode));
                        count += 1;
                    }
                    stack.push((node, path_in, depth + 1));
                }
                Kind::File if wanted => {
                    clear(&target)?;
                    write_file(img, &node, &target)?;
                    plat::fs::set_mode(&target, node.mode as u32);
                    count += 1;
                }
                Kind::Symlink if wanted => {
                    clear(&target)?;
                    if !plat::fs::symlink(&node.target, &target, false) {
                        return Err(format!("{target}: Verknüpfung nicht angelegt"));
                    }
                    count += 1;
                }
                _ => {}
            }
        }
    }
    for (path, mode) in modes.iter().rev() {
        plat::fs::set_mode(path, *mode as u32 | 0o700);
    }
    Ok(count)
}

/// a folder at path: an existing folder is fine, another kind of entry is replaced
fn make_dir(path: &str) -> Result<(), String> {
    match lstat(path) {
        Some(m) if m.is_dir() => Ok(()),
        _ => {
            clear(path)?;
            if plat::mkdir(path) { Ok(()) } else { Err(format!("{path} wurde nicht angelegt")) }
        }
    }
}

/// make room for a file or link: an old file or link goes (a link is never written through),
/// a folder in the way is an error.  The parent is a real folder: made by extract() on the
/// way down, links among the parents replaced by make_dir()
fn clear(path: &str) -> Result<(), String> {
    match lstat(path) {
        None => Ok(()),
        Some(m) if m.is_dir() => Err(format!("{path} ist ein Ordner")),
        Some(_) if plat::delete(path) => Ok(()),
        Some(_) => Err(format!("{path} lässt sich nicht ersetzen")),
    }
}

fn write_file(img: &mut Image, node: &Inode, path: &str) -> Result<(), String> {
    let f = File::open(path, FO_WRITE).ok_or_else(|| format!("{path} lässt sich nicht schreiben"))?;
    let mut off = 0u64;
    let mut ok = true;
    let read = img.read(node, &mut |b: &[u8]| {
        let mut done = 0;
        while done < b.len() {
            match f.write_at(off + done as u64, &b[done..]) {
                Some(n) if n > 0 => done += n,
                _ => {
                    ok = false;
                    return false;
                }
            }
        }
        off += b.len() as u64;
        true
    });
    match (read, ok) {
        (true, true) if off == node.size => Ok(()),
        (_, false) => Err(format!("{path}: Schreiben fehlgeschlagen")),
        _ => Err(format!("{path}: beschädigte Daten im Abbild")),
    }
}

/// a glob: * any run of bytes (also '/'), ? one byte
fn matches(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, si));
            pi += 1;
        } else if let Some((sp, ss)) = star {
            pi = sp + 1;
            si = ss + 1;
            star = Some((sp, ss + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}
