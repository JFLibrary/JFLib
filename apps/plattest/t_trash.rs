// t_trash -- plattest --test trash: plat::trash (lib/plat/trash.rs, guarded.rs)
//
//   Linux: a scratch XDG_DATA_HOME: the item leaves its place, Trash/files has it and
//   Trash/info a matching .trashinfo (Path, DeletionDate); the same name twice (a.2.txt),
//   escaped names, a folder, a vfs folder mount, mem://, missing paths, the trash itself;
//   another file system (/dev/shm, when it is one): $topdir/.Trash-$uid with a relative Path
//   Windows: one throwaway file into the real recycle bin (it must be gone from its place;
//   the bin is never emptied, nothing else is touched), missing paths, mem://
//   Both: at the end a manifest installed in this process -- trash counts as writing
// A policy stays installed: not part of "all".

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;

use plat::policy;
use plat::vfs::{self, DirFs};

use crate::{T, rm_tree};

fn text(p: &str) -> String {
    String::from_utf8(plat::read_file(p).unwrap_or_default()).unwrap_or_default()
}

// "YYYY-MM-DDThh:mm:ss"
fn date_ok(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, &c)| match i {
            4 | 7 => c == b'-',
            10 => c == b'T',
            13 | 16 => c == b':',
            _ => c.is_ascii_digit(),
        })
}

// the info file of a trash: "[Trash Info]", Path=want, a DeletionDate
fn info_ok(info: &str, want: &str) -> bool {
    let t = text(info);
    let mut l = t.lines();
    l.next() == Some("[Trash Info]") && l.next() == Some(&*format!("Path={want}")) && l.next().and_then(|d| d.strip_prefix("DeletionDate=")).is_some_and(date_ok)
}

pub fn t_trash(t: &mut T) {
    let cwd = plat::cwd().replace('\\', "/");
    let base = format!("{cwd}/build/plattest/{}-trash-{:x}", crate::OS, plat::time_us() & 0xFF_FFFF);
    t.check(plat::mkdir(&format!("{base}/work")), "trash: scratch folder");
    t.check(!plat::trash(&format!("{base}/work/missing.txt")), "trash: a missing file is no success");
    t.check(plat::write_file("mem://t.txt", b"m") && !plat::trash("mem://t.txt") && plat::file_exists("mem://t.txt") == 1,
        "trash: mem:// has no trash (the file stays)");
    if cfg!(windows) {
        windows(t, &base);
    } else {
        linux(t, &base);
    }
    installed(t, &base);
}

fn windows(t: &mut T, base: &str) {
    // one throwaway file into the real recycle bin
    let f = format!("{base}/work/jflib-plattest-trash-{:x}.txt", plat::time_us() & 0xFFFF_FFFF);
    t.check(plat::write_file(&f, b"plattest: this file may be deleted"), "trash: a throwaway file");
    t.check(plat::trash(&f), "trash: into the recycle bin");
    t.expect_i(plat::file_exists(&f) as i64, 0, "trash: the file is gone from its place");
    t.info(&format!("  ({f} is in the recycle bin)\n"));
    // by hand: jfgui_TRASH_NOBIN = a folder without a recycle bin (a network share such as
    // //wsl.localhost/Ubuntu-24.04/tmp): the shell would delete for good, trash refuses
    if let Some(dir) = plat::env("jfgui_TRASH_NOBIN").filter(|d| !d.is_empty()) {
        let g = format!("{}/jflib-plattest-nobin-{:x}.txt", dir.replace('\\', "/").trim_end_matches('/'), plat::time_us() & 0xFFFF_FFFF);
        t.check(plat::write_file(&g, b"stays"), "trash: a file in a folder without a recycle bin");
        t.check(!plat::trash(&g), "trash: ... is refused (it would be deleted for good)");
        t.check(plat::read_file(&g).as_deref() == Some(&b"stays"[..]), "trash: ... and stays where it is");
        plat::delete(&g);
    }
}

fn linux(t: &mut T, base: &str) {
    let data = format!("{base}/data");
    plat::setenv("XDG_DATA_HOME", Some(&data));
    let tr = format!("{data}/Trash");
    let a = format!("{base}/work/a.txt");
    t.check(plat::write_file(&a, b"first"), "trash: a file");
    t.check(plat::trash(&a), "trash: into a scratch XDG_DATA_HOME");
    t.expect_i(plat::file_exists(&a) as i64, 0, "trash: the file is gone from its place");
    t.check(plat::read_file(&format!("{tr}/files/a.txt")).as_deref() == Some(&b"first"[..]), "trash: Trash/files/a.txt has it");
    t.check(info_ok(&format!("{tr}/info/a.txt.trashinfo"), &a), "trash: Trash/info/a.txt.trashinfo (Path, DeletionDate)");
    t.info(&format!("  {}", text(&format!("{tr}/info/a.txt.trashinfo")).replace('\n', "\n  ")));
    let mode = plat::fs::stat(&tr).map_or(0, |m| m.mode & 0o777);
    t.info(&format!("\n  (Trash mode {mode:o})\n"));
    // the same name again
    t.check(plat::write_file(&a, b"second") && plat::trash(&a), "trash: the same name a second time");
    t.check(plat::read_file(&format!("{tr}/files/a.2.txt")).as_deref() == Some(&b"second"[..]), "trash: ... as files/a.2.txt");
    t.check(info_ok(&format!("{tr}/info/a.2.txt.trashinfo"), &a), "trash: ... with info/a.2.txt.trashinfo of the same Path");
    t.check(plat::read_file(&format!("{tr}/files/a.txt")).as_deref() == Some(&b"first"[..]), "trash: the first one is untouched");
    // an entry of files/ without an info file is not replaced
    t.check(plat::write_file(&format!("{tr}/files/a.3.txt"), b"orphan") && plat::write_file(&a, b"third") && plat::trash(&a),
        "trash: a third time, files/a.3.txt taken by an orphan");
    t.check(plat::read_file(&format!("{tr}/files/a.3.txt")).as_deref() == Some(&b"orphan"[..])
        && plat::read_file(&format!("{tr}/files/a.4.txt")).as_deref() == Some(&b"third"[..])
        && plat::file_exists(&format!("{tr}/info/a.3.txt.trashinfo")) == 0, "trash: ... it goes to a.4.txt");
    // names are URL-escaped in Path
    let odd = format!("{base}/work/b ü%.txt");
    t.check(plat::write_file(&odd, b"odd") && plat::trash(&odd), "trash: a name with a blank, an umlaut and %");
    t.check(info_ok(&format!("{tr}/info/b ü%.txt.trashinfo"), &format!("{base}/work/b%20%C3%BC%25.txt")), "trash: ... Path is URL-escaped");
    // a folder
    let dir = format!("{base}/work/folder");
    t.check(plat::mkdir(&dir) && plat::write_file(&format!("{dir}/inner.txt"), b"in") && plat::trash(&dir), "trash: a folder");
    t.check(plat::file_exists(&dir) == 0 && plat::read_file(&format!("{tr}/files/folder/inner.txt")).as_deref() == Some(&b"in"[..]),
        "trash: ... with its files");
    // a name of a folder mount, mem://, the trash itself, the root
    t.check(vfs::mount("scratch", Arc::new(DirFs::new(&format!("{base}/work")))), "trash: a DirFs mount");
    t.check(plat::write_file("scratch://v.txt", b"vfs") && plat::trash("scratch://v.txt") && plat::file_exists("scratch://v.txt") == 0,
        "trash: a vfs name of a folder mount");
    t.check(plat::file_exists(&format!("{tr}/files/v.txt")) == 1 && !plat::trash("scratch://"), "trash: ... in the trash; the mount's root is refused");
    vfs::unmount("scratch");
    t.check(!plat::trash(&format!("{tr}/files/a.txt")) && plat::file_exists(&format!("{tr}/files/a.txt")) == 1, "trash: the trash itself is refused");
    t.check(!plat::trash("/"), "trash: the root is refused");
    other_fs(t, &data);
}

// a file on another file system goes to $topdir/.Trash-$uid of its mount (/dev/shm on WSL)
fn other_fs(t: &mut T, data: &str) {
    let shm = "/dev/shm";
    let same = plat::fs::stat(shm).zip(plat::fs::stat(data)).map(|(a, b)| a.dev == b.dev);
    if same != Some(false) || plat::fs::stat("/dev").map(|m| m.dev) == plat::fs::stat(shm).map(|m| m.dev) {
        t.info("  (no /dev/shm of its own: the top folder trash is not tested)\n");
        return;
    }
    let name = format!("jflib-plattest-{:x}.txt", plat::time_us() & 0xFFFF_FFFF);
    let f = format!("{shm}/{name}");
    if !plat::write_file(&f, b"shm") {
        t.info("  (/dev/shm is not writable: the top folder trash is not tested)\n");
        return;
    }
    let uid = plat::fs::stat(&f).map_or(0, |m| m.uid);
    let top = format!("{shm}/.Trash-{uid}");
    let fresh = plat::file_exists(&top) == 0;
    t.check(plat::trash(&f), "trash: a file on another file system (/dev/shm)");
    t.expect_i(plat::file_exists(&f) as i64, 0, "trash: ... is gone from its place");
    t.check(plat::read_file(&format!("{top}/files/{name}")).as_deref() == Some(&b"shm"[..]), "trash: ... in $topdir/.Trash-$uid/files");
    t.check(info_ok(&format!("{top}/info/{name}.trashinfo"), &name), "trash: ... its Path is relative to the top folder");
    let mode = plat::fs::stat(&top).map_or(0, |m| m.mode & 0o777);
    t.check(mode == 0o700, "trash: ... $topdir/.Trash-$uid has mode 0700");
    // only our own entries go; the folders only when this test made them (and they are empty)
    plat::delete(&format!("{top}/files/{name}"));
    plat::delete(&format!("{top}/info/{name}.trashinfo"));
    if fresh {
        plat::delete(&format!("{top}/files"));
        plat::delete(&format!("{top}/info"));
        plat::delete(&top);
    }
}

// trash needs fs.write on the path
fn installed(t: &mut T, base: &str) {
    let ok = plat::mkdir(&format!("{base}/no")) && plat::mkdir(&format!("{base}/rw"))
        && plat::write_file(&format!("{base}/no/keep.txt"), b"keep") && plat::write_file(&format!("{base}/rw/go.txt"), b"go");
    t.check(ok, "trash: files for the manifest");
    let manifest = format!("fs.read {base}/no/**\nfs.write {base}/rw/**\n");
    let key = match policy::install(&manifest) {
        Ok(k) => k,
        Err(e) => {
            t.check(false, &format!("trash: install ({e})"));
            return;
        }
    };
    t.check(!plat::trash(&format!("{base}/no/keep.txt")), "trash: refused below fs.read");
    t.expect_i(plat::file_exists(&format!("{base}/no/keep.txt")) as i64, 1, "trash: ... the file stays");
    t.check(policy::denials().iter().any(|d| d.kind == "fs.write" && d.target.ends_with("keep.txt")), "trash: ... recorded as fs.write");
    if cfg!(not(windows)) {
        // (Windows: no second file into the real recycle bin)
        t.check(plat::trash(&format!("{base}/rw/go.txt")), "trash: allowed below fs.write");
    }
    key.trusted(|| rm_tree(base));
    t.check(key.trusted(|| plat::file_exists(base)) == 0, "trash: scratch tree removed");
}
