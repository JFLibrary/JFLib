// t_vfs -- plattest --test vfs: one file namespace (lib/plat/vfs.rs, guarded.rs)
//
//   names and routes (OS paths stay OS paths, "..", unknown schemes, roots), mem:// through
//   the ordinary file functions, a DirFs mount on a scratch folder (also File, FileLock,
//   base::Ini and os_path), JFA1 bundles (pack_bundle, LZ4, damaged bundles), Overlay, Sub,
//   archive::ZipFs, the app's folders (app_id, app_dir, config://); File / FileLock streams
//   on mem://, bundles, ZIP archives and a provider without streams; plat::fs on mounts
//   (stat, times, read_dir, no links); Overlay whiteouts (MemFs and DirFs on top)

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::vfs::{self, Bundle, DirFs, MemFs, Overlay, Provider, Route, Sub};
use plat::{FO_READ, FO_RW, FO_WRITE, File, FileLock};

use crate::{T, rm_tree};

fn names(path: &str) -> Vec<String> {
    let mut v = Vec::new();
    plat::dir_list(path, &mut |n, d, _| v.push(if d { format!("{n}/") } else { String::from(n) }));
    v.sort();
    v
}

pub fn t_vfs(t: &mut T) {
    routes(t);
    mem(t);
    dirfs(t);
    bundles(t);
    layers(t);
    zip(t);
    app_folders(t);
    streams(t);
    fs_names(t);
    whiteouts(t);
}

fn routes(t: &mut T) {
    let os = |n: &str| matches!(vfs::route(n), Route::Os);
    let bad = |n: &str| matches!(vfs::route(n), Route::Bad);
    t.check(os("C:/x") && os("c:\\x") && os("/a/b:c") && os("notes:2024/x") && os("rel/x"), "vfs: OS paths stay OS paths");
    t.check(bad("nosuch://x") && bad("mem://a/../b") && bad("mem://a/\0"), "vfs: unknown schemes and .. are refused");
    t.check(matches!(vfs::route("mem:"), Route::Vfs(_, ref p) if p.is_empty()), "vfs: mem: is the root");
    t.check(matches!(vfs::route("mem:/a//./b\\c"), Route::Vfs(_, ref p) if p == "a/b/c"), "vfs: paths are normalised");
    t.check(!vfs::mount("X", Arc::new(MemFs::new())) && !vfs::mount("a", Arc::new(MemFs::new())), "vfs: invalid schemes cannot be mounted");
    t.check(plat::read_file("https://example.org/").is_none(), "vfs: a URL is no file");
    t.check(vfs::provider("mem").is_some() && vfs::provider("nosuch").is_none(), "vfs: provider of a mount");
}

fn mem(t: &mut T) {
    t.check(plat::write_file("mem://a/b/c.txt", b"abc"), "vfs: mem:// write creates folders");
    t.check(plat::read_file("mem://a/b/c.txt").as_deref() == Some(&b"abc"[..]), "vfs: mem:// read");
    t.expect_i(plat::file_exists("mem://a/b") as i64, 2, "vfs: mem:// folder");
    t.expect_i(plat::file_exists("mem://a/b/c.txt") as i64, 1, "vfs: mem:// file");
    t.expect_i(plat::file_exists("mem://a/x") as i64, 0, "vfs: mem:// missing");
    t.check(plat::file_exists_checked("mem://a/x") == Some(0), "vfs: mem:// file_exists_checked");
    t.check(plat::mkdir("mem://d/e") && names("mem://") == ["a/", "d/"], "vfs: mem:// mkdir and listing");
    t.check(!plat::delete("mem://a/b"), "vfs: a folder with files is not deleted");
    t.check(plat::rename("mem://a/b/c.txt", "mem://d/c2.txt") && names("mem://d") == ["c2.txt", "e/"], "vfs: mem:// rename");
    t.check(plat::delete("mem://d/c2.txt") && plat::delete("mem://d/e") && plat::delete("mem://d"), "vfs: mem:// delete");
    t.check(!plat::write_file("mem://", b"x") && !plat::write_file("mem://a", b"x"), "vfs: no file over the root or a folder");
    t.check(!plat::rename("mem://a/b", "config://b"), "vfs: no rename across mounts");
}

fn dirfs(t: &mut T) {
    let cwd = plat::cwd().replace('\\', "/");
    let base = format!("{cwd}/build/plattest/{}-vfs-{:x}", crate::OS, plat::time_us() & 0xFF_FFFF);
    t.check(vfs::mount("scratch", Arc::new(DirFs::new(&base))), "vfs: mount a DirFs");
    t.check(plat::write_file("scratch://sub/x.txt", b"dir") && plat::file_exists(&format!("{base}/sub/x.txt")) == 1, "vfs: DirFs write lands in the folder");
    t.check(plat::read_file("scratch://sub/x.txt").as_deref() == Some(&b"dir"[..]), "vfs: DirFs read");
    t.check(names("scratch://") == ["sub/"], "vfs: DirFs listing");
    t.expect_s(vfs::os_path("scratch://sub/x.txt").as_deref(), Some(&*format!("{base}/sub/x.txt")), "vfs: os_path of a DirFs name");
    let f = File::open("scratch://sub/y.bin", FO_WRITE);
    t.check(f.as_ref().is_some_and(|f| f.write_at(0, b"stream") == Some(6)), "vfs: File::open on a DirFs");
    drop(f);
    let mut buf = [0u8; 6];
    t.check(File::open("scratch://sub/y.bin", FO_READ).and_then(|f| f.read_at(0, &mut buf)) == Some(6) && &buf == b"stream", "vfs: ... and read back");
    t.check(FileLock::acquire("scratch://LOCK").is_some(), "vfs: FileLock on a DirFs");
    let mut ini = base::Ini::new();
    ini.set_int("s", "k", 42);
    t.check(ini.save("scratch://cfg/settings.ini") && base::Ini::load("scratch://cfg/settings.ini").get_int("s", "k", 0) == 42, "vfs: base::Ini through a mount");
    t.check(plat::file_exists("scratch:") == 2 && plat::file_exists(&format!("{cwd}/scratch:")) == 0, "vfs: Ini's parent folder is the mount, not a folder 'scratch:'");
    t.check(plat::rename("scratch://sub/x.txt", "scratch://moved/x.txt") && plat::file_exists("scratch://moved/x.txt") == 1, "vfs: DirFs rename creates the folder");
    t.check(vfs::unmount("scratch") && plat::read_file("scratch://moved/x.txt").is_none(), "vfs: unmount");
    rm_tree(&base);
}

fn bundles(t: &mut T) {
    let big: Vec<u8> = b"JFLib ".iter().copied().cycle().take(6000).collect();
    let files: [(&str, &[u8]); 4] = [("b.txt", b"bee"), ("a/one.txt", b"1"), ("a/deep/two.bin", &big), ("empty", b"")];
    let packed = vfs::pack_bundle(&files);
    t.check(packed.as_ref().is_some_and(|p| p.len() < 1000), "vfs: pack_bundle compresses with LZ4");
    let Some(packed) = packed else { return };
    let Some(b) = Bundle::new(packed.clone()) else {
        t.check(false, "vfs: Bundle::new");
        return;
    };
    t.expect_i(b.len() as i64, 4, "vfs: four files in the bundle");
    t.check(b.read("a/deep/two.bin").as_deref() == Some(&big[..]) && b.read("b.txt").as_deref() == Some(&b"bee"[..]), "vfs: bundle read (LZ4 and stored)");
    t.check(b.read("empty").is_some_and(|d| d.is_empty()) && b.read("a").is_none(), "vfs: an empty file; a folder is no file");
    t.check(b.kind("") == 2 && b.kind("a") == 2 && b.kind("a/deep") == 2 && b.kind("a/d") == 0 && b.kind("b.txt") == 1, "vfs: bundle kinds");
    let mut l = Vec::new();
    b.list("a", &mut |n, d, s| l.push(format!("{n}{}{s}", if d { "/" } else { ":" })));
    t.check(l == ["deep/0", "one.txt:1"], "vfs: bundle listing");
    t.check(!b.write("x", b"y") && !b.mkdir("x") && !b.delete("b.txt"), "vfs: a bundle is read-only");
    t.check(vfs::pack_bundle(&[("a", b"1"), ("a", b"2")]).is_none() && vfs::pack_bundle(&[("../x", b"1")]).is_none(), "vfs: pack_bundle refuses duplicates and ..");
    let mut bad = packed.clone();
    bad[0] = b'X';
    let mut cut = packed.clone();
    cut.truncate(packed.len() - 3);
    let mut big_size = packed.clone();
    big_size[12 + 8 + 3] = 0x40; // the first entry's data_off points far beyond the end
    t.check(Bundle::new(bad).is_none() && Bundle::new(cut).is_none() && Bundle::new(big_size).is_none(), "vfs: damaged bundles are refused");
    t.check(Bundle::new(Vec::new()).is_some_and(|b| b.is_empty()), "vfs: an empty bundle");
}

fn layers(t: &mut T) {
    let low = Bundle::new(vfs::pack_bundle(&[("theme/a.css", b"low a"), ("theme/b.css", b"low b")]).unwrap_or_default());
    let Some(low) = low else {
        t.check(false, "vfs: bundle for the overlay");
        return;
    };
    let low: Arc<dyn Provider> = Arc::new(low);
    let top: Arc<dyn Provider> = Arc::new(MemFs::new());
    t.check(top.write("theme/a.css", b"top a"), "vfs: overlay top layer");
    vfs::mount("ov", Arc::new(Overlay::new(alloc::vec![top.clone(), low.clone()])));
    t.check(plat::read_file("ov://theme/a.css").as_deref() == Some(&b"top a"[..]), "vfs: overlay reads the top layer first");
    t.check(plat::read_file("ov://theme/b.css").as_deref() == Some(&b"low b"[..]), "vfs: ... then the lower ones");
    t.check(names("ov://theme") == ["a.css", "b.css"], "vfs: overlay lists every layer once");
    t.check(plat::write_file("ov://theme/c.css", b"new") && top.read("theme/c.css").is_some(), "vfs: overlay writes go to the top layer");
    match Sub::new(low.clone(), "theme") {
        Some(s) => {
            vfs::mount("theme", Arc::new(s));
            t.check(plat::read_file("theme://b.css").as_deref() == Some(&b"low b"[..]) && names("theme://") == ["a.css", "b.css"], "vfs: Sub shows one folder");
        }
        None => t.check(false, "vfs: Sub::new"),
    }
    t.check(Sub::new(low, "../x").is_none(), "vfs: Sub refuses ..");
    vfs::unmount("ov");
    vfs::unmount("theme");
}

fn zip(t: &mut T) {
    let entries = [
        archive::Entry { name: String::from("doc/readme.txt"), data: b"zip text".to_vec() },
        archive::Entry { name: String::from("img/a.bin"), data: alloc::vec![7u8; 3000] },
    ];
    let Ok(z) = archive::write(&entries, archive::Limits::default()) else {
        t.check(false, "vfs: archive::write");
        return;
    };
    match archive::ZipFs::new(&z, archive::Limits::default()) {
        Ok(fs) => {
            vfs::mount("pack", Arc::new(fs));
            t.check(plat::read_file("pack://doc/readme.txt").as_deref() == Some(&b"zip text"[..]), "vfs: ZipFs read");
            t.check(names("pack://") == ["doc/", "img/"] && plat::file_exists("pack://img/a.bin") == 1, "vfs: ZipFs listing");
            t.check(!plat::write_file("pack://doc/x", b"1"), "vfs: ZipFs is read-only");
            let f = File::open("pack://doc/readme.txt", FO_READ);
            t.check(f.as_ref().and_then(read_all).as_deref() == Some(&b"zip text"[..]) && File::open("pack://doc/readme.txt", FO_RW).is_none(), "vfs: a stream on a ZIP entry (read only)");
            // archive::write stamps 1980-01-01 00:00 (DOS), read as UTC
            t.check(plat::fs::stat("pack://img/a.bin").is_some_and(|m| m.is_file() && m.size == 3000 && m.mtime_ns == 315_532_800_000_000_000), "vfs: fs::stat on a ZIP entry: size and DOS time");
            vfs::unmount("pack");
        }
        Err(_) => t.check(false, "vfs: ZipFs::new"),
    }
    t.check(archive::ZipFs::new(b"not a zip at all, sorry", archive::Limits::default()).is_err(), "vfs: ZipFs refuses a damaged archive");
}

fn app_folders(t: &mut T) {
    t.expect_s(Some(vfs::app_id().as_str()), Some("plattest"), "vfs: app_id from the build (JFGUI_APP)");
    let cfg = vfs::app_dir(vfs::APP_CONFIG).unwrap_or_default();
    let want = format!("{}/plattest", plat::config_dir().unwrap_or_default());
    t.expect_s(Some(cfg.as_str()), Some(want.as_str()), "vfs: app_dir(APP_CONFIG)");
    let cache = vfs::app_dir(vfs::APP_CACHE).unwrap_or_default();
    t.check(cache.ends_with(if cfg!(windows) { "/plattest/Cache" } else { "/plattest" }), "vfs: app_dir(APP_CACHE)");
    t.expect_s(vfs::os_path("config://x.ini").as_deref(), Some(&*format!("{cfg}/x.ini")), "vfs: config:// is the app's settings folder");
    let name = format!("config://plattest-vfs-{:x}.txt", plat::time_us() & 0xFF_FFFF);
    t.check(plat::write_file(&name, b"cfg") && plat::read_file(&name).as_deref() == Some(&b"cfg"[..]) && plat::delete(&name), "vfs: write, read and delete in config://");
    t.check(plat::file_exists("app://") == 2 && names("app://").is_empty(), "vfs: app:// without assets is empty");
}

// ---------------------------------------------------------------- streams, plat::fs, whiteouts

// the whole file through read_at
fn read_all(f: &File) -> Option<Vec<u8>> {
    let n = usize::try_from(f.size()?).ok()?;
    let mut v = alloc::vec![0u8; n];
    let mut off = 0;
    while off < n {
        let k = f.read_at(off as u64, &mut v[off..])?;
        if k == 0 {
            break;
        }
        off += k;
    }
    v.truncate(off);
    Some(v)
}

// a provider with the required calls and write only: no streams, no OS files
struct Plain(Arc<MemFs>);

impl Provider for Plain {
    fn kind(&self, p: &str) -> u32 {
        self.0.kind(p)
    }
    fn read(&self, p: &str) -> Option<Vec<u8>> {
        self.0.read(p)
    }
    fn list(&self, p: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        self.0.list(p, f)
    }
    fn write(&self, p: &str, d: &[u8]) -> bool {
        self.0.write(p, d)
    }
}

fn streams(t: &mut T) {
    // mem://: the handles share the file's bytes
    let w = File::open("mem://s/a.bin", FO_WRITE);
    t.check(w.as_ref().is_some_and(|f| f.write_at(0, b"hello") == Some(5) && f.write_at(7, b"!") == Some(1)), "vfs: File::open(FO_WRITE) on mem:// creates and writes");
    t.check(plat::read_file("mem://s/a.bin").as_deref() == Some(&b"hello\0\0!"[..]), "vfs: ... read_file sees the writes at once (a gap is zeros)");
    let r = File::open("mem://s/a.bin", FO_READ);
    t.check(r.as_ref().and_then(read_all).as_deref() == Some(&b"hello\0\0!"[..]), "vfs: a second handle reads them");
    t.check(r.as_ref().is_some_and(|f| f.write_at(0, b"x").is_none() && !f.set_len(1)), "vfs: an FO_READ handle cannot write");
    t.check(w.as_ref().is_some_and(|f| f.set_len(5) && f.size() == Some(5)) && r.as_ref().and_then(|f| f.size()) == Some(5), "vfs: set_len, seen by both handles");
    t.check(File::open("mem://s/a.bin", FO_RW).is_some_and(|f| read_all(&f).as_deref() == Some(&b"hello"[..])), "vfs: FO_RW keeps the contents");
    t.check(plat::write_file("mem://s/a.bin", b"new") && r.as_ref().and_then(read_all).as_deref() == Some(&b"hello"[..]), "vfs: write_file replaces the name, an open handle keeps its file");
    t.check(File::open("mem://s/a.bin", FO_WRITE).is_some_and(|f| f.size() == Some(0)) && plat::read_file("mem://s/a.bin").is_some_and(|d| d.is_empty()), "vfs: FO_WRITE truncates");
    drop((w, r));
    t.check(File::open("mem://s/none", FO_READ).is_none() && File::open("mem://s", FO_RW).is_none() && File::open("mem://", FO_WRITE).is_none(), "vfs: no stream on a missing file, a folder or the root");
    t.check(File::open("mem://s/a.bin/x", FO_WRITE).is_none() && File::open("mem://s/a.bin", 7).is_none(), "vfs: no file below a file, no unknown mode");
    t.check(File::create_new("mem://s/a.bin").is_none() && File::create_new("mem://s/b.bin").is_some_and(|f| f.write_at(0, b"b") == Some(1)), "vfs: File::create_new on mem://");
    let tmp = File::open_temp(Some("mem://s"));
    t.check(tmp.as_ref().is_some_and(|f| f.write_at(0, b"tmp") == Some(3) && read_all(f).as_deref() == Some(&b"tmp"[..])), "vfs: File::open_temp in a mount without OS files");
    t.check(names("mem://s") == ["a.bin", "b.bin"] && File::open_temp(Some("mem://nosuch")).is_none(), "vfs: ... it has no name; it needs a folder");
    drop(tmp);
    let l = FileLock::acquire("mem://s/LOCK");
    t.check(l.is_some() && FileLock::acquire("mem://s/LOCK").is_none(), "vfs: FileLock on mem:// (busy while held)");
    drop(l);
    t.check(FileLock::acquire("mem://s/LOCK").is_some() && FileLock::acquire("mem://s").is_none(), "vfs: ... free again after drop; no lock on a folder");
    for n in ["a.bin", "b.bin", "LOCK"] {
        plat::delete(&format!("mem://s/{n}"));
    }
    t.check(plat::delete("mem://s"), "vfs: the stream test's files are gone");

    // a bundle: a stored file in place, an LZ4 one unpacked
    let big: Vec<u8> = (0..20000u32).map(|i| (i % 7) as u8).collect();
    let files: [(&str, &[u8]); 2] = [("big.bin", &big), ("small.txt", b"small")];
    match vfs::pack_bundle(&files).and_then(Bundle::new) {
        Some(b) => {
            vfs::mount("bnd", Arc::new(b));
            let f = File::open("bnd://big.bin", FO_READ);
            let mut part = [0u8; 10];
            t.check(f.as_ref().is_some_and(|f| f.size() == Some(20000) && f.read_at(19995, &mut part) == Some(5) && part[..5] == big[19995..]), "vfs: a stream on an LZ4-packed bundle file");
            t.check(File::open("bnd://small.txt", FO_READ).and_then(|f| read_all(&f)).as_deref() == Some(&b"small"[..]), "vfs: ... and on a stored one");
            t.check(plat::fs::stat("bnd://big.bin").is_some_and(|m| m.size == 20000 && m.mtime_ns == 0), "vfs: fs::stat on a bundle file (no time)");
            t.check(File::open("bnd://small.txt", FO_RW).is_none() && File::open("bnd://n", FO_WRITE).is_none() && FileLock::acquire("bnd://n").is_none(), "vfs: a bundle stays read-only");
            vfs::unmount("bnd");
        }
        None => t.check(false, "vfs: a bundle for the streams"),
    }

    // a provider without streams of its own: the whole file, written back
    let inner = Arc::new(MemFs::new());
    vfs::mount("plain", Arc::new(Plain(inner.clone())));
    let f = File::open("plain://p.txt", FO_RW);
    t.check(f.as_ref().is_some_and(|f| f.write_at(0, b"plain") == Some(5)), "vfs: File::open on a provider with read / write only");
    t.check(f.as_ref().is_some_and(|f| f.sync()) && inner.read("p.txt").as_deref() == Some(&b"plain"[..]), "vfs: ... sync writes the file back");
    t.check(f.as_ref().is_some_and(|f| f.write_at(5, b"!") == Some(1)), "vfs: ... written again");
    drop(f);
    t.check(inner.read("p.txt").as_deref() == Some(&b"plain!"[..]), "vfs: ... and dropping it writes it back");
    t.check(File::open("plain://p.txt", FO_READ).and_then(|f| read_all(&f)).as_deref() == Some(&b"plain!"[..]), "vfs: ... read as a stream");
    vfs::unmount("plain");
}

fn fs_names(t: &mut T) {
    use plat::fs;
    t.check(plat::write_file("mem://f/x.txt", b"12345"), "vfs: a file for plat::fs");
    t.check(fs::stat("mem://f/x.txt").is_some_and(|m| m.is_file() && m.size == 5 && m.mtime_ns > 0), "vfs: fs::stat on mem:// (kind, size, time)");
    t.check(fs::stat("mem://f").is_some_and(|m| m.is_dir()) && fs::stat("mem://f/none").is_none() && fs::lstat("mem://f/x.txt").is_some_and(|m| m.is_file()), "vfs: ... a folder, a missing name, lstat");
    let when = 1_600_000_000_123_456_789i64;
    t.check(fs::set_times("mem://f/x.txt", when - 5, when) && fs::stat("mem://f/x.txt").is_some_and(|m| m.mtime_ns == when && m.atime_ns == when - 5), "vfs: fs::set_times on mem://");
    t.check(fs::read_dir("mem://f").is_some_and(|v| v.len() == 1 && v[0].name == "x.txt" && v[0].kind == fs::FileKind::File), "vfs: fs::read_dir on mem://");
    t.check(fs::read_link("mem://f/x.txt").is_none() && !fs::symlink("x.txt", "mem://f/l", false) && !fs::hard_link("mem://f/x.txt", "mem://f/h"), "vfs: mounts without OS files have no links");
    t.check(fs::write_file_private("mem://f/p.txt", b"p") && fs::mkdir_private("mem://f/d") && fs::sync_dir("mem://f") && fs::free_space("mem://f").is_none(), "vfs: write_file_private, mkdir_private, sync_dir, free_space");
    for n in ["mem://f/x.txt", "mem://f/p.txt", "mem://f/d", "mem://f"] {
        plat::delete(n);
    }
    // a folder mount is its OS folder
    let cwd = plat::cwd().replace('\\', "/");
    let base = format!("{cwd}/build/plattest/{}-vfsfs-{:x}", crate::OS, plat::time_us() & 0xFF_FFFF);
    vfs::mount("fsdir", Arc::new(DirFs::new(&base)));
    t.check(plat::write_file("fsdir://a.txt", b"abc"), "vfs: a file in a DirFs mount");
    let os = fs::stat(&format!("{base}/a.txt"));
    t.check(os.is_some() && fs::stat("fsdir://a.txt") == os, "vfs: fs::stat on a folder mount is the OS file's");
    let secs = |m: fs::Meta| m.mtime_ns.div_euclid(1_000_000_000);
    t.check(fs::set_times("fsdir://a.txt", when, when) && fs::stat(&format!("{base}/a.txt")).map(secs) == Some(when / 1_000_000_000), "vfs: fs::set_times on a folder mount");
    t.check(fs::hard_link("fsdir://a.txt", "fsdir://b.txt") && plat::read_file(&format!("{base}/b.txt")).as_deref() == Some(&b"abc"[..]), "vfs: fs::hard_link inside a folder mount");
    t.check(fs::read_dir("fsdir://").is_some_and(|v| v.len() == 2) && fs::free_space("fsdir://").is_some(), "vfs: fs::read_dir and free_space on a folder mount");
    vfs::unmount("fsdir");
    rm_tree(&base);
}

fn whiteouts(t: &mut T) {
    let files: [(&str, &[u8]); 5] = [("t/a.css", b"low a"), ("t/b.css", b"low b"), ("d/x.txt", b"x"), ("d/y.txt", b"y"), ("s/r.txt", b"read")];
    let Some(low) = vfs::pack_bundle(&files).and_then(Bundle::new) else {
        t.check(false, "vfs: a bundle for the whiteouts");
        return;
    };
    let low: Arc<dyn Provider> = Arc::new(low);
    let top = Arc::new(MemFs::new());
    vfs::mount("wh", Arc::new(Overlay::new(alloc::vec![top.clone() as Arc<dyn Provider>, low.clone()])));
    t.check(plat::delete("wh://t/b.css"), "vfs: an overlay deletes a lower layer's file");
    t.check(plat::file_exists("wh://t/b.css") == 0 && plat::read_file("wh://t/b.css").is_none() && names("wh://t") == ["a.css"], "vfs: ... it is gone (kind, read, listing)");
    t.check(top.kind("t/.wh.b.css") == 1 && !plat::delete("wh://t/b.css"), "vfs: ... a whiteout in the top layer; a second delete fails");
    t.check(plat::write_file("wh://t/b.css", b"again") && plat::read_file("wh://t/b.css").as_deref() == Some(&b"again"[..]) && top.kind("t/.wh.b.css") == 0, "vfs: writing it again removes the whiteout");
    t.check(!plat::write_file("wh://t/.wh.a.css", b"x") && plat::file_exists("wh://t/.wh.b.css") == 0, "vfs: the markers' names are reserved");
    // a folder: its files first, then the folder; made again it shows no lower files
    t.check(!plat::delete("wh://d"), "vfs: a lower folder with files is not deleted");
    t.check(plat::delete("wh://d/x.txt") && plat::delete("wh://d/y.txt") && plat::delete("wh://d"), "vfs: ... its files, then the folder");
    t.check(plat::file_exists("wh://d") == 0 && names("wh://") == ["s/", "t/"], "vfs: the folder is gone");
    t.check(plat::write_file("wh://d/z.txt", b"z") && names("wh://d") == ["z.txt"] && plat::file_exists("wh://d/x.txt") == 0, "vfs: a folder made again is opaque");
    // rename: a lower file copied up, its old name hidden
    t.check(plat::rename("wh://t/a.css", "wh://t/c.css") && plat::read_file("wh://t/c.css").as_deref() == Some(&b"low a"[..]) && plat::file_exists("wh://t/a.css") == 0, "vfs: rename of a lower layer's file");
    t.check(names("wh://t") == ["b.css", "c.css"] && File::open("wh://t/a.css", FO_READ).is_none(), "vfs: ... the listing; no stream on the old name");
    // a stream: FO_RW copies the lower file up, the lower layer stays as it is
    let f = File::open("wh://s/r.txt", FO_RW);
    t.check(f.as_ref().is_some_and(|f| f.write_at(4, b"!") == Some(1)) && top.read("s/r.txt").as_deref() == Some(&b"read!"[..]) && low.read("s/r.txt").as_deref() == Some(&b"read"[..]), "vfs: File::open(FO_RW) on an overlay copies the file up");
    drop(f);
    t.check(plat::fs::set_times("wh://t/c.css", 7, 9) && plat::fs::stat("wh://t/c.css").is_some_and(|m| m.mtime_ns == 9), "vfs: fs::set_times through an overlay");
    vfs::unmount("wh");

    // a DirFs on top keeps its whiteouts on disk
    let cwd = plat::cwd().replace('\\', "/");
    let base = format!("{cwd}/build/plattest/{}-vfswh-{:x}", crate::OS, plat::time_us() & 0xFF_FFFF);
    let on_disk = || Arc::new(Overlay::new(alloc::vec![Arc::new(DirFs::new(&base)) as Arc<dyn Provider>, low.clone()]));
    vfs::mount("whd", on_disk());
    t.check(plat::delete("whd://t/a.css") && plat::file_exists(&format!("{base}/t/.wh.a.css")) == 1, "vfs: a DirFs top layer keeps the whiteout as a file");
    vfs::mount("whd", on_disk());
    t.check(plat::file_exists("whd://t/a.css") == 0 && names("whd://t") == ["b.css"], "vfs: ... a new overlay on the folder still hides it");
    // (Windows: the OS file can be read once the writing handle is closed)
    let wrote = File::open("whd://t/b.css", FO_RW).is_some_and(|f| f.write_at(0, b"L") == Some(1));
    t.check(wrote && plat::read_file(&format!("{base}/t/b.css")).as_deref() == Some(&b"Low b"[..]), "vfs: FO_RW on a DirFs top layer copies up to the OS file");
    vfs::unmount("whd");
    rm_tree(&base);
}
