// t_bookmarks -- (iOS) the bookmarks that keep files chosen in the system's picker readable after a
// new start (lib/plat/apple_ios_services.rs)
//
//   plattest --test bookmarks   files in a scratch folder of the temporary directory stand in for
//                               chosen ones (plat::ios_bookmark_add, what the picker does with its
//                               URL): kept newest first, a path chosen again moves to the front
//                               once; what the launch does (plat::ios_bookmarks_restore) keeps them,
//                               finds a renamed file under its new path and drops a deleted one;
//                               at most 100 are kept.  The kept list of the test app is cleared
//                               before and after.
//
// Elsewhere it says so and passes.

use crate::T;

#[cfg(not(target_os = "ios"))]
pub fn t_bookmarks(t: &mut T) {
    t.info("  (bookmarks: iOS only, the picker's security-scoped files)\n");
}

#[cfg(target_os = "ios")]
pub fn t_bookmarks(t: &mut T) {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    use plat::{ios_bookmark_add, ios_bookmark_paths, ios_bookmarks_clear, ios_bookmarks_restore};

    let Some(tmp) = plat::temp_dir() else {
        t.check(false, "bookmarks: a temporary directory");
        return;
    };
    let dir = format!("{}/jfgui-bookmarks-test", tmp.trim_end_matches('/'));
    let file = |n: &str| format!("{dir}/{n}");
    let _ = plat::fs::mkdir_private(&dir);
    for n in ["a.txt", "b.txt"] {
        t.check(plat::fs::write_file_private(&file(n), n.as_bytes()), &format!("bookmarks: {n} made"));
    }
    ios_bookmarks_clear();
    t.check(ios_bookmark_paths().is_empty(), "bookmarks: none kept after clearing");

    t.check(ios_bookmark_add(&file("a.txt")) && ios_bookmark_add(&file("b.txt")), "bookmarks: two files kept");
    t.check(ios_bookmark_paths() == [file("b.txt"), file("a.txt")], "bookmarks: newest first");
    t.check(ios_bookmark_add(&file("a.txt")), "bookmarks: a chosen again");
    t.check(ios_bookmark_paths() == [file("a.txt"), file("b.txt")], "bookmarks: a in front, once");

    ios_bookmarks_restore();
    t.check(ios_bookmark_paths() == [file("a.txt"), file("b.txt")], "bookmarks: the launch keeps both");

    t.check(plat::rename(&file("b.txt"), &file("c.txt")), "bookmarks: b renamed to c");
    t.check(plat::delete(&file("a.txt")), "bookmarks: a deleted");
    ios_bookmarks_restore();
    let now = ios_bookmark_paths();
    t.check(now == [file("c.txt")], &format!("bookmarks: the launch finds c, drops a ({now:?})"));

    let many: Vec<String> = (0..105).map(|k| file(&format!("m{k}.txt"))).collect();
    let mut ok = true;
    for f in &many {
        ok &= plat::fs::write_file_private(f, b"m") && ios_bookmark_add(f);
    }
    let now = ios_bookmark_paths();
    t.check(ok && now.len() == 100 && now.first() == many.last(), &format!("bookmarks: at most 100 kept, the newest first ({})", now.len()));

    ios_bookmarks_clear();
    for f in many.iter().chain([file("c.txt")].iter()) {
        let _ = plat::delete(f);
    }
    let _ = plat::delete(&dir);
}
