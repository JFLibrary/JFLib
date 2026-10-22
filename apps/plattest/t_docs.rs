// t_docs -- plattest --test docs-bus (Linux): lasting document IDs of Flatpak's documents portal
// through plat::persistent_name (lib/plat/linux_docs.rs), against tools/platshared/docs_bus.py
// (a fake portal on a private bus) or, with --mode real, xdg-document-portal itself (a private
// session; its FUSE mount in a scratch $XDG_RUNTIME_DIR).
//
// --doc names a document below $XDG_RUNTIME_DIR/doc (brief.txt).  The fake makes a lasting
// document of it (another ID, the same bytes, the same answer when asked again); the real
// portal answers a document of its own mount with its own ID (a lasting one stays what it is, a
// session one cannot become lasting from the app's side), --lasting names one it made lasting.
// Ordinary paths, a document ID alone, a missing file and the portal's refusal come back
// unchanged.  --phase 2 (the portal stopped and started again): the lasting document read and
// written, the session's one gone.  --mode chooser: see chooser() below.

#[cfg(target_os = "linux")]
use alloc::{format, string::String};

use crate::T;

#[cfg(windows)]
pub fn t_docs_bus(_: &mut T) {
    plat::write_stderr(b"plattest: --test docs-bus is Linux only\n");
    plat::exit(2);
}

#[cfg(target_os = "linux")]
pub fn t_docs_bus(t: &mut T) {
    let args = base::Args(plat::args());
    let mode = String::from(args.value("--mode").unwrap_or("fake"));
    let phase = String::from(args.value("--phase").unwrap_or("1"));
    let Some(run) = plat::env("XDG_RUNTIME_DIR") else {
        t.check(false, "docs: XDG_RUNTIME_DIR set by the caller");
        return;
    };
    let base = format!("{run}/doc");
    let text = |p: &str| plat::read_file(p).and_then(|b| String::from_utf8(b).ok());
    let doc = String::from(args.value("--doc").unwrap_or(""));
    let lasting = String::from(args.value("--lasting").unwrap_or(""));
    t.info(&format!("  mode {mode}, phase {phase}, mount {base}\n"));
    if mode == "chooser" {
        chooser(t, &args, &phase, &base);
        return;
    }
    if phase == "2" {
        // after a restart of the portal: the lasting document still reaches the file
        t.check(text(&lasting).as_deref() == Some("Brief vom Host\n"), "docs: a lasting document after the portal's restart");
        t.check(plat::write_file(&lasting, b"geschrieben nach dem Neustart\n"), "docs: written through it (sibling and rename in the document's folder)");
        t.check(text(&lasting).as_deref() == Some("geschrieben nach dem Neustart\n"), "docs: read back");
        t.check(plat::file_exists(&doc) == 0, "docs: the session's document is gone");
        return;
    }
    t.check(doc.starts_with(&format!("{base}/")) && text(&doc).as_deref() == Some("Brief vom Host\n"), "docs: the document is readable");
    let kept = plat::persistent_name(&doc);
    let id = kept.strip_prefix(&format!("{base}/")).and_then(|r| r.split_once('/')).map(|(id, _)| String::from(id)).unwrap_or_default();
    if mode == "fake" {
        t.check(kept != doc && kept.ends_with("/brief.txt") && !id.is_empty(), &format!("docs: a lasting document of the file ({kept})"));
    } else {
        t.check(kept == doc, "docs: the real portal answers a document of its mount with its own ID");
    }
    t.check(text(&kept).as_deref() == Some("Brief vom Host\n"), "docs: the same bytes under that name");
    t.check(plat::persistent_name(&doc) == kept, "docs: the same file asked again: the same ID");
    t.check(plat::persistent_name(&kept) == kept, "docs: the answer stays itself");
    if !lasting.is_empty() {
        t.check(plat::persistent_name(&lasting) == lasting, "docs: a lasting document stays itself");
    }
    plat::write_stdout(format!("KEPT {kept}\n").as_bytes());
    for (p, what) in [
        (String::from("/etc/hostname"), "an ordinary path"),
        (format!("{base}/{id}"), "a document ID without a file name"),
        (format!("{base}/{id}/fehlt/x.txt"), "a path inside a document"),
        (format!("{base}/zz9/fehlt.txt"), "a missing document"),
        (String::from("mem://brief.txt"), "a vfs name"),
    ] {
        t.check(plat::persistent_name(&p) == p, &format!("docs: {what} stays unchanged"));
    }
    if mode == "fake" {
        // the fake refuses Add for this one
        let refused = format!("{base}/refused1/brief.txt");
        t.check(plat::persistent_name(&refused) == refused, "docs: the portal's refusal: unchanged");
    }
}

// --mode chooser, inside a sandbox (bwrap with a /.flatpak-info; docs_bus.py --real): the
// portal's file dialog (xdg-desktop-portal, a fake chooser behind it) hands out a document;
// a manifest with fs.user only reads it without any grant (a document of the mount inside the
// sandbox), phase 2 (the document portal restarted) reads and writes it again without a dialog
#[cfg(target_os = "linux")]
fn chooser(t: &mut T, args: &base::Args, phase: &str, base: &str) {
    let text = |p: &str| plat::read_file(p).and_then(|b| String::from_utf8(b).ok());
    let _key = plat::policy::install("fs.user\n");
    t.check(plat::policy::active(), "chooser: a manifest with fs.user only");
    t.check(plat::read_file("/etc/hostname").is_none(), "chooser: other files refused");
    if phase == "2" {
        let chosen = String::from(args.value("--chosen").unwrap_or(""));
        t.check(text(&chosen).as_deref() == Some("Brief vom Host\n"), "chooser: after the portal's restart: read without a dialog");
        t.check(plat::write_file(&chosen, b"im Sandkasten gespeichert\n"), "chooser: and written");
        return;
    }
    let mut path = String::new();
    let (r, _) = plat::file_dialog(plat::FD_OPEN, &mut path, &["txt=Text"]);
    t.check(r == 1 && path.starts_with(&format!("{base}/")) && path.ends_with("/gewaehlt.txt"), &format!("chooser: the dialog hands out a document of the mount ({path})"));
    t.check(text(&path).as_deref() == Some("Brief vom Host\n"), "chooser: read with fs.user, no grant");
    t.check(plat::persistent_name(&path) == path, "chooser: the dialog's document keeps its ID");
    plat::write_stdout(format!("CHOSEN {path}\n").as_bytes());
}
