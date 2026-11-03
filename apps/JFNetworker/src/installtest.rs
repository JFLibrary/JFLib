// installtest -- checks of installing courses without a window (part of --selftest, group "install"): the list
// and each refusal with its exact message, a fake source that fails half way, cancels and resumes, an update that
// fails and keeps the old course, what a crash leaves, uninstalling with the progress kept, versions, the setting, a
// folder as the source, the list of courses after a change; and with --academy-dir (tests/model.uis names
// apps/JFNetworker/academy) every file against index.json, every course as jfn-sim course sees it, the sample
// course from its folder end to end.  Everything is written to mem://, nothing touches the network: the only
// connection tried goes to a host no rule names, which is refused before it starts.  The real HTTP path is
// tools/academy_e2e.py's.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use jfn_model::academy::*;
use jfn_run::install::{self, Counters, Failure, Folder, Kind, Phase, Source, Web};

use crate::app::prefs::Prefs;
use crate::selftest::T;
use crate::workspace::academy::install::{failure_text, first_install_of, size_text, ASK_REASON};
use crate::workspace::academy::store;
use crate::workspace::academy::{Academy, LabStep, View};

fn s(x: &str) -> String {
    String::from(x)
}

fn sha(b: &[u8]) -> String {
    crypto::hex(&crypto::sha256(b))
}

// a course.json with one lesson
fn course_text(version: u32, lesson: &str) -> String {
    format!(
        r#"{{"format": "jfn-course", "version": 1, "course_version": {}, "title": ["Kurs", "Course"], "summary": ["Kurz", "Short"], "level": "beginner", "minutes": 5,
"units": [{{"title": ["Eins", "One"], "steps": [{{"id": "a", "kind": "lesson", "title": ["A", "A"], "file": "{}"}}]}}]}}"#,
        version, lesson
    )
}

// what a test's source holds of a course
fn entry_of(id: &str, version: u32, files: &[(&str, &[u8])]) -> IndexCourse {
    IndexCourse {
        id: s(id),
        title: jfn_model::lab::Text::new("Kurs", "Course"),
        summary: jfn_model::lab::Text::new("Kurz", "Short"),
        level: Level::Beginner,
        minutes: 5,
        version,
        sample: false,
        files: files.iter().map(|(p, b)| FileEntry { path: s(p), size: b.len() as u64, sha256: sha(b) }).collect(),
    }
}

// a good course in version v: its list entry and its files
fn good(id: &str, v: u32) -> (IndexCourse, Vec<(String, Vec<u8>)>) {
    let course = course_text(v, "a.md").into_bytes();
    let de = format!("Lektion, Version {}\n", v).into_bytes();
    let en = format!("Lesson, version {}\n", v).into_bytes();
    let files: [(&str, &[u8]); 3] = [("course.json", &course), ("de/a.md", &de), ("en/a.md", &en)];
    (entry_of(id, v, &files), files.iter().map(|(p, b)| (s(p), b.to_vec())).collect())
}

/// a source in memory that does what a test tells it
#[derive(Default)]
struct Fake {
    files: BTreeMap<String, Vec<u8>>,
    /// what was asked for, and how much was there at that moment
    asked: RefCell<Vec<(String, u64)>>,
    /// this file breaks off after so many bytes, once
    break_at: RefCell<Option<(String, u64)>>,
    /// this file breaks off with nothing new, every time
    dead: RefCell<Option<String>>,
    /// this file stops the worker after so many bytes
    cancel_at: RefCell<Option<(String, u64)>>,
    index_bytes: Vec<u8>,
    /// how often it was asked for the list
    lists: Cell<u32>,
}

impl Fake {
    fn with(id: &str, files: &[(String, Vec<u8>)]) -> Fake {
        let mut f = Fake::default();
        f.add(id, files);
        f
    }

    fn add(&mut self, id: &str, files: &[(String, Vec<u8>)]) {
        for (p, b) in files {
            self.files.insert(format!("{}/{}", id, p), b.clone());
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.borrow().iter().map(|x| x.0.clone()).collect()
    }
}

impl Source for Fake {
    fn index(&self) -> Result<Vec<u8>, Failure> {
        self.lists.set(self.lists.get() + 1);
        Ok(self.index_bytes.clone())
    }

    fn fetch(&self, id: &str, f: &FileEntry, part: &str, c: &Counters) -> Result<(), Failure> {
        let key = format!("{}/{}", id, f.path);
        let have = plat::fs::stat(part).map_or(0, |m| m.size);
        self.asked.borrow_mut().push((f.path.clone(), have));
        if self.dead.borrow().as_deref() == Some(f.path.as_str()) {
            return Err(Failure { kind: Kind::Interrupted, msg: jfn_model::msg!("Die Verbindung brach bei {} von {} Bytes ab.", have, f.size), problems: Vec::new() });
        }
        let data = self.files.get(&key).cloned().unwrap_or_default();
        let start = (have as usize).min(data.len());
        let mut end = data.len();
        let mut broke = false;
        if let Some((p, at)) = self.break_at.borrow().clone()
            && p == f.path
        {
            end = end.min(at as usize);
            broke = true;
        }
        let mut stop = false;
        if let Some((p, at)) = self.cancel_at.borrow().clone()
            && p == f.path
        {
            end = end.min(at as usize);
            stop = true;
        }
        let file = plat::File::open(part, plat::FO_RW).ok_or_else(|| Failure { kind: Kind::Disk, msg: jfn_model::msg!("{} lässt sich nicht schreiben.", part), problems: Vec::new() })?;
        if end > start {
            file.write_at(start as u64, &data[start..end]);
        }
        file.sync();
        c.file_bytes(end as u64);
        if broke {
            *self.break_at.borrow_mut() = None;
            return Err(Failure { kind: Kind::Interrupted, msg: jfn_model::msg!("Die Verbindung brach bei {} von {} Bytes ab.", end, f.size), problems: Vec::new() });
        }
        if stop {
            *self.cancel_at.borrow_mut() = None;
            c.stop();
            return Err(Failure { kind: Kind::Cancelled, msg: jfn_model::msg!("Abgebrochen."), problems: Vec::new() });
        }
        Ok(())
    }
}

// a fresh folder below mem://
fn fresh(t: &mut T, name: &str) -> String {
    let root = format!("mem://jfn-install/{}", name);
    t.check(&format!("{}: a new folder", name), plat::file_exists(&root) == 0 && plat::mkdir(&root));
    root
}

fn names(dir: &str) -> Vec<String> {
    let mut v: Vec<String> = plat::fs::read_dir(dir).unwrap_or_default().into_iter().map(|e| e.name).filter(|n| n != ".install-lock").collect();
    v.sort();
    v
}

fn text(path: &str) -> String {
    plat::read_file(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

// the version of what stands in a course's folder
fn version_at(root: &str, id: &str) -> Option<u32> {
    Course::from_text(&text(&format!("{}/{}/course.json", root, id))).ok().map(|c| c.course_version)
}

// a failure's kind and exact German message
fn said(r: &Result<(), Failure>) -> Option<(Kind, String)> {
    r.as_ref().err().map(|e| (e.kind, e.msg.text()))
}

fn index_said(r: Result<Index, Failure>) -> Option<(Kind, String)> {
    r.err().map(|e| (e.kind, e.msg.text()))
}

// a list with one course whose single file has this path, size and checksum
fn one_file(path: &str, size: u64, hash: &str) -> Vec<u8> {
    format!(
        r#"{{"format": "jfn-academy", "version": 1, "courses": [{{"id": "k", "title": ["K", "K"], "version": 1,
"files": [{{"path": "course.json", "size": 1, "sha256": "{0}"}}, {{"path": "{1}", "size": {2}, "sha256": "{3}"}}]}}]}}"#,
        "a".repeat(64),
        path,
        size,
        hash
    )
    .into_bytes()
}

// ---------------------------------------------------------------- the list

fn index_t(t: &mut T) {
    let hash = "b".repeat(64);
    let i = install::parse_index(&one_file("de/a.md", 7, &hash));
    t.check("a good list reads", i.as_ref().is_ok_and(|x| x.courses.len() == 1 && x.courses[0].files.len() == 2 && x.courses[0].total() == 8 && !x.courses[0].sample));
    let cases: [(&str, Vec<u8>, Kind, String); 13] = [
        ("..", one_file("../geheim.md", 1, &hash), Kind::BadIndex, s("Kurs k: Der Pfad „../geheim.md“ führt mit „..“ aus dem Ordner des Kurses.")),
        ("deep ..", one_file("de/../../geheim.md", 1, &hash), Kind::BadIndex, s("Kurs k: Der Pfad „de/../../geheim.md“ führt mit „..“ aus dem Ordner des Kurses.")),
        ("an absolute path", one_file("/etc/passwd", 1, &hash), Kind::BadIndex, s("Kurs k: Der Pfad „/etc/passwd“ ist absolut; erlaubt sind nur Pfade im Ordner des Kurses.")),
        ("a drive", one_file("C:/x.md", 1, &hash), Kind::BadIndex, s("Kurs k: Der Pfad „C:/x.md“ ist absolut; erlaubt sind nur Pfade im Ordner des Kurses.")),
        ("a hidden part", one_file("de/.x", 1, &hash), Kind::BadIndex, s("Kurs k: Der Pfad „de/.x“ hat einen leeren oder unzulässigen Teil.")),
        ("a device name", one_file("con.md", 1, &hash), Kind::BadIndex, s("Kurs k: Der Pfad „con.md“ hat einen leeren oder unzulässigen Teil.")),
        ("a file too large", one_file("big.bin", FILE_MAX + 1, &hash), Kind::TooLarge, format!("Kurs k: die Datei big.bin ist mit {} Bytes größer als die erlaubten {} Bytes.", FILE_MAX + 1, FILE_MAX)),
        ("no checksum", one_file("a.md", 1, "abc"), Kind::BadIndex, s("Kurs k: die Prüfsumme von a.md ist kein SHA-256.")),
        ("the same file twice", one_file("COURSE.JSON", 1, &hash), Kind::BadIndex, s("Kurs k: die Datei COURSE.JSON kommt doppelt vor.")),
        ("not UTF-8", alloc::vec![0xFF, 0xFE], Kind::BadIndex, s("Die Kursliste ist kein UTF-8-Text.")),
        ("another format", br#"{"format": "jfn-course", "version": 1}"#.to_vec(), Kind::BadIndex, s("Das ist keine Kursliste.")),
        ("a newer version", br#"{"format": "jfn-academy", "version": 9}"#.to_vec(), Kind::BadIndex, s("Die Datei hat die Version 9; dieses Programm liest Version 1 bis 1.")),
        ("too long", alloc::vec![b' '; INDEX_MAX + 1], Kind::TooLarge, format!("Die Kursliste ist größer als die erlaubten {} Bytes.", INDEX_MAX)),
    ];
    for (name, bytes, kind, msg) in cases {
        t.eq(&format!("refused, {}", name), index_said(install::parse_index(&bytes)), Some((kind, msg)));
    }
    // limits of a course and of the list
    let mut big = entry_of("k", 1, &[("course.json", b"x")]);
    for k in 0..COURSE_BYTES_MAX / FILE_MAX + 1 {
        big.files.push(FileEntry { path: format!("f{}.bin", k), size: FILE_MAX, sha256: "c".repeat(64) });
    }
    let list = Index { format: s(INDEX_FORMAT), version: 1, courses: alloc::vec![big.clone()] };
    t.eq("a course too large", list.issues().iter().map(|p| p.msg.text()).collect::<Vec<_>>(), alloc::vec![format!("Kurs k: er ist mit {} Bytes größer als die erlaubten {} Bytes.", big.total(), COURSE_BYTES_MAX)]);
    let root = fresh(t, "limits");
    let c = Counters::default();
    let f = Fake::default();
    t.eq("install refuses it too, and nothing was asked for", install::install(&f, &root, &big, &c).map_err(|e| e.kind), Err(Kind::TooLarge));
    t.eq("  nothing asked", f.asked(), Vec::<String>::new());
    let many = Index { format: s(INDEX_FORMAT), version: 1, courses: (0..INDEX_COURSES_MAX + 1).map(|k| entry_of(&format!("k{}", k), 1, &[("course.json", b"x")])).collect() };
    t.eq("too many courses", many.issues().first().map(|p| p.msg.text()), Some(format!("Die Kursliste nennt mehr als {} Kurse.", INDEX_COURSES_MAX)));
    let mut files = entry_of("k", 1, &[("course.json", b"x")]);
    files.files.extend((0..COURSE_FILES_MAX).map(|k| FileEntry { path: format!("f{}", k), size: 1, sha256: "c".repeat(64) }));
    let list = Index { format: s(INDEX_FORMAT), version: 1, courses: alloc::vec![files] };
    t.eq("too many files", list.issues().first().map(|p| p.msg.text()), Some(format!("Kurs k: die Kursliste nennt mehr als {} Dateien.", COURSE_FILES_MAX)));
    let nocourse = Index { format: s(INDEX_FORMAT), version: 1, courses: alloc::vec![entry_of("k", 1, &[("a.md", b"x")])] };
    t.eq("no course.json", nocourse.issues().first().map(|p| p.msg.text()), Some(s("Kurs k: die Kursliste nennt keine Datei course.json.")));
    // a bad id: it would be a folder
    let bad = Index { format: s(INDEX_FORMAT), version: 1, courses: alloc::vec![entry_of("../k", 1, &[("course.json", b"x")])] };
    t.eq("an id that would leave the folder", bad.issues().first().map(|p| p.msg.text()), Some(s("Die Kennung „../k“ darf nur aus kleinen Buchstaben, Ziffern, - und _ bestehen.")));
    // what the read gives back is what came, so that it can be kept
    let mut f = Fake::default();
    f.index_bytes = one_file("a.md", 1, &hash);
    let read = install::read_index(&f);
    t.check("read_index: the list and its bytes", read.is_ok_and(|(i, b)| i.courses.len() == 1 && b == f.index_bytes));
    t.eq("  asked once", f.lists.get(), 1);
}

// ---------------------------------------------------------------- a course's way

fn install_t(t: &mut T) {
    let root = fresh(t, "plain");
    let (e, files) = good("k", 1);
    let f = Fake::with("k", &files);
    let c = Counters::default();
    t.eq("a course goes in", said(&install::install(&f, &root, &e, &c)), None);
    t.eq("  its folder, nothing else", names(&root), alloc::vec![s("k")]);
    t.eq("  every file", names(&format!("{}/k", root)), alloc::vec![s("course.json"), s("de"), s("en")]);
    t.eq("  its bytes", plat::read_file(&format!("{}/k/de/a.md", root)), Some(files[1].1.clone()));
    t.eq("  asked once each", f.asked(), alloc::vec![s("course.json"), s("de/a.md"), s("en/a.md")]);
    let seen = c.seen();
    t.eq("  the counters at the end", (seen.phase, seen.file, seen.files, seen.done == seen.total, seen.total), (Phase::Placing, 3, 3, true, e.total()));
    let (lines, errors) = jfn_run::check_course(&format!("{}/k", root));
    t.eq("  jfn-sim course finds nothing fatal", (errors, lines.iter().any(|l| l.starts_with("Fehler"))), (0, false));
    let list = store::list(&root);
    t.check("  the list of courses knows it, and only it", list.len() == 1 && list[0].usable().is_some() && list[0].bytes == e.total());
    // once more: nothing comes again, the same version stands
    let f2 = Fake::with("k", &files);
    t.eq("the same version again", said(&install::install(&f2, &root, &e, &Counters::default())), None);
    t.eq("  every file again: the folder started empty", f2.asked().len(), 3);
    t.eq("  in place", version_at(&root, "k"), Some(1));

    // a file the list does not name is never asked for, and what the course then lacks stops it
    let root = fresh(t, "unlisted");
    let course = course_text(1, "fehlt.md").into_bytes();
    let e = entry_of("k", 1, &[("course.json", &course)]);
    let mut f = Fake::default();
    f.files.insert(s("k/course.json"), course.clone());
    f.files.insert(s("k/de/fehlt.md"), b"steht in der Quelle, aber nicht in der Liste".to_vec());
    let r = install::install(&f, &root, &e, &Counters::default());
    t.eq("a course that needs an unlisted file", said(&r), Some((Kind::BadCourse, s("Der Kurs k hat 1 Fehler."))));
    t.eq("  what is wrong", r.err().map(|e| e.problems.iter().map(|m| m.text()).collect::<Vec<_>>()), Some(alloc::vec![s("Schritt a: die Lektion fehlt.md fehlt in jeder Sprache.")]));
    t.eq("  never asked for", f.asked(), alloc::vec![s("course.json")]);
    t.eq("  nothing stays", names(&root), Vec::<String>::new());

    // and a file that is in the source and not on the list does not come along
    let root = fresh(t, "extra");
    let (e, files) = good("k", 1);
    let mut f = Fake::with("k", &files);
    f.files.insert(s("k/de/geheim.md"), b"nicht in der Liste".to_vec());
    t.eq("a source with one file more", said(&install::install(&f, &root, &e, &Counters::default())), None);
    t.eq("  it stays there", names(&format!("{}/k/de", root)), alloc::vec![s("a.md")]);

    // the list's version must be the course's
    let root = fresh(t, "versions");
    let (mut e, files) = good("k", 1);
    e.version = 2;
    let f = Fake::with("k", &files);
    t.eq("another version in the file than in the list", said(&install::install(&f, &root, &e, &Counters::default())), Some((Kind::BadCourse, s("Die Kursdatei nennt die Version 1, die Kursliste die Version 2."))));
    t.eq("  nothing stays", names(&root), Vec::<String>::new());

    // the wrong bytes: size and checksum are this pipeline's own, whatever the source does
    let root = fresh(t, "wrong");
    let (e, files) = good("k", 1);
    let mut f = Fake::with("k", &files);
    f.files.insert(s("k/de/a.md"), b"zu kurz".to_vec());
    let r = install::install(&f, &root, &e, &Counters::default());
    t.eq("the wrong size", said(&r), Some((Kind::Checksum, format!("Die Datei de/a.md hat 7 Bytes; die Kursliste nennt {} Bytes.", files[1].1.len()))));
    t.eq("  it did not stay", plat::file_exists(&format!("{}/.k.part/de/a.md", root)), 0);
    t.eq("  no course", plat::file_exists(&format!("{}/k", root)), 0);
    let root = fresh(t, "wrong2");
    let mut f = Fake::with("k", &files);
    let mut same = files[1].1.clone();
    same[0] ^= 1;
    f.files.insert(s("k/de/a.md"), same);
    let r = install::install(&f, &root, &e, &Counters::default());
    t.eq("the wrong checksum", said(&r), Some((Kind::Checksum, s("Die Prüfsumme der Datei de/a.md stimmt nicht mit der Kursliste überein."))));
    t.eq("  it did not stay", plat::file_exists(&format!("{}/.k.part/de/a.md", root)), 0);

    // too large while it comes: a source that sends more than the list says
    let root = fresh(t, "more");
    let mut f = Fake::with("k", &files);
    let mut longer = files[2].1.clone();
    longer.extend_from_slice(b"und noch mehr");
    f.files.insert(s("k/en/a.md"), longer.clone());
    t.eq("more bytes than the list says", said(&install::install(&f, &root, &e, &Counters::default())), Some((Kind::Checksum, format!("Die Datei en/a.md hat {} Bytes; die Kursliste nennt {} Bytes.", longer.len(), files[2].1.len()))));

    // two at once
    let root = fresh(t, "busy");
    let lock = plat::FileLock::acquire(&format!("{}/.install-lock", root));
    t.check("the lock is free at first", lock.is_some());
    t.eq("a second installation waits for nobody", said(&install::install(&Fake::with("k", &files), &root, &e, &Counters::default())), Some((Kind::Busy, s("Eine andere Installation läuft gerade in diesem Ordner."))));
    t.eq("uninstalling neither", said(&install::uninstall(&root, "k")), Some((Kind::Busy, s("Eine andere Installation läuft gerade in diesem Ordner."))));
    drop(lock);
    t.eq("  then it goes", said(&install::install(&Fake::with("k", &files), &root, &e, &Counters::default())), None);
}

fn resume_t(t: &mut T) {
    let (e, files) = good("k", 1);
    let half = files[1].1.len() as u64 / 2;

    // a transfer that breaks off goes on by itself while something arrives
    let root = fresh(t, "resume");
    let f = Fake::with("k", &files);
    *f.break_at.borrow_mut() = Some((s("de/a.md"), half));
    t.eq("it breaks off half way and goes on", said(&install::install(&f, &root, &e, &Counters::default())), None);
    t.eq("  the file twice, from where it stopped", f.asked.borrow().iter().filter(|x| x.0 == "de/a.md").map(|x| x.1).collect::<Vec<_>>(), alloc::vec![0, half]);
    t.eq("  the bytes are right", plat::read_file(&format!("{}/k/de/a.md", root)), Some(files[1].1.clone()));

    // the way is gone: what came stays, the next attempt goes on and asks only for the rest
    let root = fresh(t, "gone");
    let f = Fake::with("k", &files);
    *f.dead.borrow_mut() = Some(s("de/a.md"));
    let r = install::install(&f, &root, &e, &Counters::default());
    t.eq("the way is gone", r.as_ref().err().map(|x| x.kind), Some(Kind::Interrupted));
    t.check("  a started installation", install::has_part(&root, "k"));
    t.eq("  no course", plat::file_exists(&format!("{}/k", root)), 0);
    t.eq("  and the list does not see the part", store::list(&root).len(), 0);
    let f = Fake::with("k", &files);
    t.eq("the next attempt", said(&install::install(&f, &root, &e, &Counters::default())), None);
    t.eq("  asks only for what is missing", f.asked(), alloc::vec![s("de/a.md"), s("en/a.md")]);
    t.check("  and no part stays", !install::has_part(&root, "k"));

    // cancelled in the middle of a file
    let root = fresh(t, "cancel");
    let f = Fake::with("k", &files);
    *f.cancel_at.borrow_mut() = Some((s("de/a.md"), half));
    let c = Counters::default();
    let r = install::install(&f, &root, &e, &c);
    t.eq("cancelled", said(&r), Some((Kind::Cancelled, s("Abgebrochen."))));
    t.eq("  it did not go on", f.asked(), alloc::vec![s("course.json"), s("de/a.md")]);
    t.eq("  no course", plat::file_exists(&format!("{}/k", root)), 0);
    t.eq("  half a file stays for the next time", plat::fs::stat(&format!("{}/.k.part/de/a.md", root)).map(|m| m.size), Some(half));
    let f = Fake::with("k", &files);
    t.eq("goes on after that", said(&install::install(&f, &root, &e, &Counters::default())), None);
    t.eq("  from where it was", f.asked.borrow().clone(), alloc::vec![(s("de/a.md"), half), (s("en/a.md"), 0)]);

    // a stop before anything happens
    let root = fresh(t, "cancel0");
    let c = Counters::default();
    c.stop();
    let f = Fake::with("k", &files);
    t.eq("cancelled before the start", said(&install::install(&f, &root, &e, &c)), Some((Kind::Cancelled, s("Abgebrochen."))));
    t.eq("  nothing asked", f.asked(), Vec::<String>::new());

    // a part from another version: its start does not fit, so once from nothing
    let root = fresh(t, "stale");
    plat::write_file(&format!("{}/.k.part/de/a.md", root), b"Fremd");
    plat::write_file(&format!("{}/.k.part/alt/weg.md", root), b"die Liste nennt das nicht mehr");
    let f = Fake::with("k", &files);
    t.eq("a part that does not fit", said(&install::install(&f, &root, &e, &Counters::default())), None);
    t.eq("  once on, once from nothing", f.asked.borrow().iter().filter(|x| x.0 == "de/a.md").map(|x| x.1).collect::<Vec<_>>(), alloc::vec![5, 0]);
    t.eq("  the bytes are right", plat::read_file(&format!("{}/k/de/a.md", root)), Some(files[1].1.clone()));
    t.eq("  what the list does not name is gone", names(&format!("{}/k", root)), alloc::vec![s("course.json"), s("de"), s("en")]);

    // a right file in the part stays, a longer one goes
    let root = fresh(t, "keep");
    plat::write_file(&format!("{}/.k.part/course.json", root), &files[0].1);
    let mut long = files[2].1.clone();
    long.extend_from_slice(b"mehr");
    plat::write_file(&format!("{}/.k.part/en/a.md", root), &long);
    let f = Fake::with("k", &files);
    t.eq("what is there and right", said(&install::install(&f, &root, &e, &Counters::default())), None);
    t.check("  is not asked for", !f.asked().contains(&s("course.json")));
    t.eq("  and a longer file ends right", plat::read_file(&format!("{}/k/en/a.md", root)), Some(files[2].1.clone()));

    // the part of a course the list no longer names
    let root = fresh(t, "forget");
    plat::write_file(&format!("{}/.alt.part/a", root), b"x");
    plat::write_file(&format!("{}/.k.part/a", root), b"x");
    let list = Index { format: s(INDEX_FORMAT), version: 1, courses: alloc::vec![e.clone()] };
    install::forget_parts(&root, &list);
    t.eq("forget_parts: what the list does not name", names(&root), alloc::vec![s(".k.part")]);
}

fn update_t(t: &mut T) {
    let root = fresh(t, "update");
    let (e1, f1) = good("k", 1);
    t.eq("version 1", said(&install::install(&Fake::with("k", &f1), &root, &e1, &Counters::default())), None);
    plat::write_file(&format!("{}/k/de/alt.md", root), b"gibt es in Version 2 nicht");
    let (e2, f2) = good("k", 2);
    let keep = plat::read_file(&format!("{}/k/de/a.md", root));

    // fails half way
    let f = Fake::with("k", &f2);
    *f.dead.borrow_mut() = Some(s("en/a.md"));
    let r = install::install(&f, &root, &e2, &Counters::default());
    t.eq("an update that fails half way", r.as_ref().err().map(|x| x.kind), Some(Kind::Interrupted));
    t.eq("  the old course is untouched", (version_at(&root, "k"), plat::read_file(&format!("{}/k/de/a.md", root))), (Some(1), keep.clone()));
    t.check("  and usable", store::list(&root).iter().any(|x| x.id == "k" && x.usable().is_some()));

    // cancelled
    let f = Fake::with("k", &f2);
    *f.cancel_at.borrow_mut() = Some((s("en/a.md"), 2));
    t.eq("an update that is cancelled", install::install(&f, &root, &e2, &Counters::default()).err().map(|x| x.kind), Some(Kind::Cancelled));
    t.eq("  the old course is untouched", version_at(&root, "k"), Some(1));

    // a wrong file
    let mut f = Fake::with("k", &f2);
    f.files.insert(s("k/en/a.md"), b"falsch".to_vec());
    t.eq("an update with a wrong file", install::install(&f, &root, &e2, &Counters::default()).err().map(|x| x.kind), Some(Kind::Checksum));
    t.eq("  the old course is untouched", version_at(&root, "k"), Some(1));

    // a bad course
    let bad = course_text(3, "fehlt.md").into_bytes();
    let e3 = entry_of("k", 3, &[("course.json", &bad)]);
    let mut f = Fake::default();
    f.files.insert(s("k/course.json"), bad.clone());
    t.eq("an update that is no good course", install::install(&f, &root, &e3, &Counters::default()).err().map(|x| x.kind), Some(Kind::BadCourse));
    t.eq("  the old course is untouched", (version_at(&root, "k"), plat::file_exists(&format!("{}/k/de/alt.md", root))), (Some(1), 1));
    t.check("  nothing of it stays", !install::has_part(&root, "k"));

    // and the one that works: the new folder as a whole
    t.eq("the update that works", said(&install::install(&Fake::with("k", &f2), &root, &e2, &Counters::default())), None);
    t.eq("  the new version", version_at(&root, "k"), Some(2));
    t.eq("  what version 2 has not is gone", plat::file_exists(&format!("{}/k/de/alt.md", root)), 0);
    t.eq("  nothing beside it", names(&root), alloc::vec![s("k")]);

    // what a crash leaves
    let root = fresh(t, "crash");
    plat::write_file(&format!("{}/.a.old/course.json", root), b"a");
    plat::write_file(&format!("{}/.b.old/course.json", root), b"alt");
    plat::write_file(&format!("{}/b/course.json", root), b"neu");
    plat::write_file(&format!("{}/.c.trash/x/y", root), b"weg");
    plat::write_file(&format!("{}/.d.part/x", root), b"bleibt");
    plat::write_file(&format!("{}/.e.other/x", root), b"nicht von hier");
    install::recover(&root);
    t.eq("a crash: what is settled", names(&root), alloc::vec![s(".d.part"), s(".e.other"), s("a"), s("b")]);
    t.eq("  an old version without its new one comes back", text(&format!("{}/a/course.json", root)), s("a"));
    t.eq("  an old version beside its new one goes", text(&format!("{}/b/course.json", root)), s("neu"));

    // the place fails: the old course comes back.  An old folder that cannot be removed is tried again later
    let root = fresh(t, "place");
    let (e1, f1) = good("k", 1);
    t.eq("  in place", said(&install::install(&Fake::with("k", &f1), &root, &e1, &Counters::default())), None);
    plat::write_file(&format!("{}/.k.old/rest", root), b"von einem Absturz");
    t.eq("an update with an old leftover", said(&install::install(&Fake::with("k", &f2), &root, &e2, &Counters::default())), None);
    t.eq("  nothing beside it", names(&root), alloc::vec![s("k")]);
}

fn uninstall_t(t: &mut T) {
    let root = fresh(t, "remove");
    let (e, files) = good("k", 1);
    t.eq("a course goes in", said(&install::install(&Fake::with("k", &files), &root, &e, &Counters::default())), None);
    plat::write_file(&format!("{}/.k.part/x", root), b"ein angefangenes Update");
    let progress_file = format!("{}/progress.json", root);
    let mut p = Progress::default();
    p.mark_done("k", "a", 1000);
    p.record("k", "a", 80, 1000);
    t.check("the learner's file", store::write_progress(&progress_file, &p));
    t.eq("it goes out", said(&install::uninstall(&root, "k")), None);
    t.eq("  only the progress stays", names(&root), alloc::vec![s("progress.json")]);
    let kept = store::read_progress(&progress_file);
    t.check("  and says what it said", kept.is_done("k", "a") && kept.best_of("k", "a") == Some(80));
    t.eq("again: nothing to do", said(&install::uninstall(&root, "k")), None);
    t.eq("an id that would leave the folder", said(&install::uninstall(&root, "../k")), Some((Kind::BadCourse, s("Die Kennung „../k“ darf nur aus kleinen Buchstaben, Ziffern, - und _ bestehen."))));
    t.eq("it comes back", said(&install::install(&Fake::with("k", &files), &root, &e, &Counters::default())), None);
    let list = store::list(&root);
    let back = list.first().and_then(|x| x.usable().cloned());
    t.check("  with what was done", back.is_some_and(|c| store::read_progress(&progress_file).done_count(&c, "k") == 1));
}

fn folder_t(t: &mut T) {
    // a folder source in memory: the same way, its own refusals
    let src = fresh(t, "source");
    let root = fresh(t, "from-folder");
    let (e, files) = good("k", 1);
    for (p, b) in &files {
        plat::write_file(&format!("{}/k/{}", src, p), b);
    }
    plat::write_file(&format!("{}/k/de/geheim.md", src), b"nicht in der Liste");
    let folder = Folder::new(&src);
    t.eq("a folder without a list", index_said(install::read_index(&folder).map(|x| x.0)), Some((Kind::BadIndex, format!("Im Ordner {} liegt keine index.json. Autoren erzeugen sie mit tools/academy_index.py.", src))));
    t.eq("a course from a folder", said(&install::install(&folder, &root, &e, &Counters::default())), None);
    t.eq("  what the list names", names(&format!("{}/k/de", root)), alloc::vec![s("a.md")]);
    plat::write_file(&format!("{}/k/de/a.md", src), b"anders");
    let root2 = fresh(t, "from-folder2");
    t.eq("a file of another size", said(&install::install(&folder, &root2, &e, &Counters::default())), Some((Kind::Checksum, format!("Die Datei de/a.md hat 6 Bytes; die Kursliste nennt {} Bytes.", files[1].1.len()))));
    let mut same = files[1].1.clone();
    same[0] ^= 1;
    plat::write_file(&format!("{}/k/de/a.md", src), &same);
    t.eq("a file with other bytes", said(&install::install(&folder, &root2, &e, &Counters::default())), Some((Kind::Checksum, s("Die Prüfsumme der Datei de/a.md stimmt nicht mit der Kursliste überein."))));
    plat::delete(&format!("{}/k/de/a.md", src));
    t.eq("a file that is gone", said(&install::install(&folder, &root2, &e, &Counters::default())), Some((Kind::Missing, s("Die Datei de/a.md fehlt in der Quelle."))));
    // cancelled: the folder's way stops too
    plat::write_file(&format!("{}/k/de/a.md", src), &files[1].1);
    let root3 = fresh(t, "from-folder3");
    let c = Counters::default();
    c.stop();
    t.eq("a folder source can be stopped", said(&install::install(&folder, &root3, &e, &c)), Some((Kind::Cancelled, s("Abgebrochen."))));
    // a list too large in a folder is not read
    plat::write_file(&format!("{}/index.json", src), &alloc::vec![b' '; INDEX_MAX + 1]);
    t.eq("a list too large", index_said(install::read_index(&folder).map(|x| x.0)), Some((Kind::TooLarge, format!("Die Kursliste ist größer als die erlaubten {} Bytes.", INDEX_MAX))));
}

// ---------------------------------------------------------------- versions, words, the setting

fn words_t(t: &mut T) {
    t.eq("not here", standing(None, Some(2)), Standing::NotInstalled);
    t.eq("not here, not listed", standing(None, None), Standing::NotInstalled);
    t.eq("the same", standing(Some(2), Some(2)), Standing::Current);
    t.eq("a newer one is listed", standing(Some(1), Some(2)), Standing::Update);
    t.eq("here is newer", standing(Some(3), Some(2)), Standing::Newer);
    t.eq("not listed", standing(Some(2), None), Standing::Unlisted);
    t.eq("no version at all counts as the oldest", standing(Some(0), Some(1)), Standing::Update);

    // what a first install takes
    let (mut a, _) = good("a", 1);
    let (mut b, _) = good("b", 2);
    let (mut sample, _) = good("beispiel", 1);
    sample.sample = true;
    a.sample = false;
    b.sample = false;
    let list = Index { format: s(INDEX_FORMAT), version: 1, courses: alloc::vec![a, b, sample.clone()] };
    t.eq("a first install: no samples", first_install_of(&list, &[]), alloc::vec![s("a"), s("b")]);
    t.eq("  what is current stays out, a newer one comes", first_install_of(&list, &[(s("a"), 1), (s("b"), 1)]), alloc::vec![s("b")]);
    t.eq("  all here", first_install_of(&list, &[(s("a"), 1), (s("b"), 2)]), Vec::<String>::new());
    let only = Index { format: s(INDEX_FORMAT), version: 1, courses: alloc::vec![sample] };
    t.eq("  a list with only samples: its samples", first_install_of(&only, &[]), alloc::vec![s("beispiel")]);

    // every failure has its words and says what to do, each its own
    let kinds = [Kind::Address, Kind::Offline, Kind::Interrupted, Kind::Denied, Kind::OtherHost, Kind::Tls, Kind::Missing, Kind::BadIndex, Kind::Checksum, Kind::TooLarge, Kind::BadCourse, Kind::Disk, Kind::Busy, Kind::Cancelled];
    let mut titles = Vec::new();
    for k in kinds {
        let (sev, title, text) = failure_text(&Failure { kind: k, msg: jfn_model::msg::Msg::plain("Genau das."), problems: Vec::new() });
        t.check(&format!("{}: its words", k.word()), !title.is_empty() && text.len() > 20);
        t.eq(&format!("{}: an error or a plain note", k.word()), sev, if k == Kind::Cancelled { ui::IB_INFO } else { ui::IB_ERROR });
        titles.push(title);
    }
    titles.sort();
    titles.dedup();
    t.eq("each kind has words of its own", titles.len(), kinds.len());
    let (_, _, text) = failure_text(&Failure { kind: Kind::BadCourse, msg: jfn_model::msg::Msg::plain("Kurs k."), problems: (0..5).map(|k| jfn_model::msg::Msg::plain(format!("Fehler {}", k))).collect() });
    t.check("a bad course: the first problems, not a wall of them", text.contains("Fehler 2") && !text.contains("Fehler 3"));

    t.eq("sizes", [size_text(412), size_text(2048), size_text(1536), size_text(12 << 10), size_text(3 << 20), size_text((3 << 20) + (1 << 19))], [s("412 B"), s("2 KB"), s("1,5 KB"), s("12 KB"), s("3 MB"), s("3,5 MB")]);

    // what the app may do: the question comes with its reason; the program without a window has its own rules
    let rules = include_str!("../PERMISSIONS");
    t.check("PERMISSIONS: the courses' host is asked about, with the reason of the catalogue", rules.lines().any(|l| l.starts_with("ask net.connect raw.githubusercontent.com:443") && l.contains(ASK_REASON)));
    t.check("PERMISSIONS: no connection without a question", rules.lines().all(|l| !l.starts_with("net.connect")));
    t.check("PERMISSIONS.jfn-sim: it cannot ask", include_str!("../PERMISSIONS.jfn-sim").lines().any(|l| l.starts_with("net.connect raw.githubusercontent.com:443")));
    // the setting: its default, and where it leads
    let mut p = Prefs::new();
    t.eq("the address by default", p.academy_base.as_str(), install::DEFAULT_BASE);
    p.set_academy_base("https://raw.githubusercontent.com/Anders/Kurse/main/");
    t.eq("another repository: the same host", p.academy_base.as_str(), "https://raw.githubusercontent.com/Anders/Kurse/main/");
    t.check("  an address", Web::new(&p.academy_base).is_ok());
    p.reset_academy_base();
    t.eq("the way back", p.academy_base.as_str(), install::DEFAULT_BASE);
    t.check("  a course's host is the rule's", install::DEFAULT_BASE.starts_with(&format!("https://{}/", install::COURSE_HOST)));
    t.eq("no address", Web::new("kein Server").err().map(|e| (e.kind, e.msg.text())), Some((Kind::Address, s("„kein Server“ ist keine Adresse für Kurse (http:// oder https://)."))));
    t.eq("another scheme", Web::new("ftp://example.org/").err().map(|e| e.kind), Some(Kind::Address));
    // another host: refused by PERMISSIONS before anything leaves the program, and said so
    let web = Web::new("https://example.org/kurse").ok();
    let got = web.map(|w| w.index().err().map(|e| (e.kind, e.msg.text())));
    t.eq("another host", got, Some(Some((Kind::OtherHost, s("Der Host example.org ist nicht erlaubt; Kurse kommen nur von raw.githubusercontent.com.")))));
}

// ---------------------------------------------------------------- what AK2 left open

fn author_t(t: &mut T) {
    use crate::workspace::author::diagnosis_item;
    let text = text_of_fixture();
    let p = match jfn_model::Project::from_text(text) {
        Ok(p) => p,
        Err(e) => {
            t.check(&format!("the project: {}", e), false);
            return;
        }
    };
    t.eq("no cause", diagnosis_item("  ", 2, &p).err(), Some(s("Nennen Sie ein Gerät oder die beiden Enden eines Kabels: R1 oder PC1 - SW1.")));
    t.eq("three ends", diagnosis_item("a - b - c", 2, &p).err(), Some(s("Nennen Sie ein Gerät oder die beiden Enden eines Kabels: R1 oder PC1 - SW1.")));
    t.eq("no such device", diagnosis_item("XX", 2, &p).err(), Some(s("Das Gerät XX gibt es im Projekt nicht.")));
    t.eq("no such end", diagnosis_item("R1 - XX", 2, &p).err(), Some(s("Das Gerät XX gibt es im Projekt nicht.")));
    t.eq("no such cable", diagnosis_item("PC1 - PC2", 2, &p).err(), Some(s("Zwischen PC1 und PC2 gibt es kein Kabel.")));
    let dev = diagnosis_item("r1", 3, &p);
    t.check("a device, by its own name", dev.as_ref().is_ok_and(|i| i.dev == "R1" && i.id != 0 && i.points == 3 && i.peer.is_empty() && i.kind() == Some(jfn_model::lab::ItemKind::Diagnosis)));
    let cable = diagnosis_item("PC1 - R1", 2, &p);
    t.check("a cable", cable.as_ref().is_ok_and(|i| i.dev == "PC1" && i.peer == "R1" && i.peer_id != 0));
    // the simulator's grading takes them as they are
    let Ok(e) = jfn_sim::Engine::from_project(&p) else {
        t.check("the engine", false);
        return;
    };
    let (Ok(dev), Ok(cable)) = (dev, cable) else { return };
    let r1 = p.devices.iter().find(|d| d.name == "R1").map_or(0, |d| d.id);
    t.check("the right device counts", jfn_sim::assess::pick_right(&dev, &p, &e, &jfn_sim::assess::Pick::Dev(r1)));
    t.check("another does not", !jfn_sim::assess::pick_right(&cable, &p, &e, &jfn_sim::assess::Pick::Dev(r1)));
    let l = p.links.iter().find(|l| [l.a.dev, l.b.dev].contains(&r1) && [l.a.dev, l.b.dev].contains(&cable.id));
    t.check("the right cable counts", l.is_some_and(|l| jfn_sim::assess::pick_right(&cable, &p, &e, &jfn_sim::assess::Pick::Cable(l.a.clone(), l.b.clone()))));
    // leaving asks only while a test is under way
    let mut a = Academy::new();
    t.check("no test, nothing to ask about", !crate::workspace::academy::quiz::running(&a));
    a.quizzes.push(crate::workspace::academy::quiz::blank_page());
    t.check("practice has nothing to lose", !crate::workspace::academy::quiz::running(&a));
}

// a project with a router between two PCs
fn text_of_fixture() -> &'static str {
    include_str!("../tests/fixtures/lan_router.jfnet")
}

// ---------------------------------------------------------------- the list of courses after a change

fn relist_t(t: &mut T) {
    let root = fresh(t, "relist");
    for id in ["a", "b", "c"] {
        let (e, files) = good(id, 1);
        t.eq(&format!("course {}", id), said(&install::install(&Fake::with(id, &files), &root, &e, &Counters::default())), None);
    }
    let mut a = Academy::new();
    a.setup(true, Some(root.clone()));
    a.ensure();
    t.eq("three courses", a.entries.len(), 3);
    a.on = true;
    a.cur = 1;
    a.view = View::Lesson(0);
    a.lab = Some(LabStep { course: 2, step: 0, marked: true });

    // another course changes: the page stays, its numbers follow
    t.eq("a goes", said(&install::uninstall(&root, "a")), None);
    t.check("  the page stays", !a.relist(&[s("a")]));
    t.eq("  on the lesson", (a.view, a.course_id()), (View::Lesson(0), "b"));
    t.eq("  and the lab follows", a.lab.as_ref().map(|l| (l.course, l.step, l.marked)), Some((1, 0, true)));

    // the course that shows changes: back to its outline, with a note
    let (e, files) = good("b", 2);
    t.eq("b changes", said(&install::install(&Fake::with("b", &files), &root, &e, &Counters::default())), None);
    t.check("  the page moved", a.relist(&[s("b")]));
    t.eq("  to its outline", (a.view, a.course_id()), (View::Course, "b"));
    t.eq("  it is the new one", a.course().map(|c| c.course_version), Some(2));

    // the course that shows goes
    t.eq("b goes", said(&install::uninstall(&root, "b")), None);
    t.check("  the page moved", a.relist(&[s("b")]));
    t.eq("  to the list", a.view, View::Home);
    t.eq("  the lab of the other course stays", a.lab.as_ref().map(|l| l.course), Some(0));

    // the lab's course goes: the project stays, the bar goes
    t.eq("c goes", said(&install::uninstall(&root, "c")), None);
    t.check("  no page depended on it", !a.relist(&[s("c")]));
    t.check("  no bar any more", a.lab.is_none());
    t.eq("  an empty list", a.entries.len(), 0);

    // a lab whose step is gone in the new version
    let root = fresh(t, "relist-step");
    let (e, files) = good("k", 1);
    t.eq("k", said(&install::install(&Fake::with("k", &files), &root, &e, &Counters::default())), None);
    let mut a = Academy::new();
    a.setup(true, Some(root.clone()));
    a.ensure();
    a.lab = Some(LabStep { course: 0, step: 0, marked: false });
    let other = br#"{"format": "jfn-course", "version": 1, "course_version": 2, "title": ["K", "K"], "units": [{"title": ["E", "E"], "steps": [{"id": "anders", "kind": "lesson", "title": ["A", "A"], "file": "a.md"}]}]}"#;
    let e2 = entry_of("k", 2, &[("course.json", other), ("de/a.md", b"x")]);
    let mut f = Fake::default();
    f.files.insert(s("k/course.json"), other.to_vec());
    f.files.insert(s("k/de/a.md"), b"x".to_vec());
    t.eq("k changes its steps", said(&install::install(&f, &root, &e2, &Counters::default())), None);
    a.relist(&[s("k")]);
    t.check("  the lab's step is gone: no bar", a.lab.is_none());

    // quizzes hold numbers of the old list
    let root = fresh(t, "relist-quiz");
    let (e, files) = good("k", 1);
    t.eq("k", said(&install::install(&Fake::with("k", &files), &root, &e, &Counters::default())), None);
    let mut a = Academy::new();
    a.setup(true, Some(root));
    a.ensure();
    a.quizzes.push(crate::workspace::academy::quiz::blank_page());
    a.relist(&[]);
    t.check("the list again clears the quizzes", a.quizzes.is_empty());
}

// ---------------------------------------------------------------- the repository's courses

// every file below a folder, relative, sorted
fn files_below(dir: &str, rel: &str, out: &mut Vec<String>) {
    for e in plat::fs::read_dir(dir).unwrap_or_default() {
        let name = if rel.is_empty() { e.name.clone() } else { format!("{}/{}", rel, e.name) };
        if e.kind == plat::fs::FileKind::Dir {
            files_below(&format!("{}/{}", dir, e.name), &name, out);
        } else {
            out.push(name);
        }
    }
}

fn repo_t(t: &mut T, root: &str) {
    let bytes = plat::read_file(&format!("{}/index.json", root));
    t.check(&format!("{}/index.json is there (python tools/academy_index.py)", root), bytes.is_some());
    let Some(bytes) = bytes else { return };
    t.check("index.json: LF, no byte order mark", !bytes.contains(&b'\r') && !bytes.starts_with(b"\xEF\xBB\xBF"));
    let index = match install::parse_index(&bytes) {
        Ok(i) => i,
        Err(e) => {
            t.check(&format!("index.json: {}", e.msg), false);
            return;
        }
    };
    let ids: Vec<&str> = index.courses.iter().map(|c| c.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    t.eq("index.json: stable order of its courses", ids.clone(), sorted);
    let folders: Vec<String> = store::list(root).into_iter().map(|e| e.id).collect();
    t.eq("every folder is in the index, every entry has its folder", folders, ids.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    t.check("the sample is listed, as a sample", index.courses.iter().any(|c| c.id == "beispiel" && c.sample));
    for c in &index.courses {
        let dir = format!("{}/{}", root, c.id);
        let mut found = Vec::new();
        files_below(&dir, "", &mut found);
        found.sort();
        let mut listed: Vec<String> = c.files.iter().map(|f| f.path.clone()).collect();
        let order = listed.clone();
        listed.sort();
        t.eq(&format!("{}: stable order of its files", c.id), order, listed.clone());
        t.eq(&format!("{}: every file is listed, every listed file is there (a course changed without a new index? python tools/academy_index.py)", c.id), found, listed);
        for f in &c.files {
            let data = plat::read_file(&format!("{}/{}", dir, f.path)).unwrap_or_default();
            t.eq(&format!("{}/{}: its size in the index", c.id, f.path), data.len() as u64, f.size);
            t.eq(&format!("{}/{}: its checksum in the index", c.id, f.path), sha(&data), f.sha256.clone());
        }
        let (_, errors) = jfn_run::check_course(&dir);
        t.eq(&format!("{}: jfn-sim course finds no errors", c.id), errors, 0);
        let own = Course::from_text(&text(&format!("{}/{}", dir, COURSE_FILE)));
        match own {
            Ok(x) => {
                t.eq(&format!("{}: the index's version is course_version", c.id), c.version, x.course_version);
                t.eq(&format!("{}: its title", c.id), c.title.clone(), x.title.clone());
                t.eq(&format!("{}: its summary", c.id), c.summary.clone(), x.summary.clone());
                t.eq(&format!("{}: its level and time", c.id), (c.level, c.minutes, c.sample), (x.level, x.minutes, x.sample));
            }
            Err(m) => t.check(&format!("{}: {}", c.id, m), false),
        }
    }
    // the sample from its folder, through the same way, end to end
    let target = fresh(t, "sample");
    let folder = Folder::new(root);
    let Ok((list, _)) = install::read_index(&folder) else {
        t.check("the folder's list reads", false);
        return;
    };
    let Some(sample) = list.courses.iter().find(|c| c.id == "beispiel") else { return };
    let c = Counters::default();
    t.eq("the sample from its folder", said(&install::install(&folder, &target, sample, &c)), None);
    let seen = c.seen();
    t.eq("  every file came", (seen.file, seen.files, seen.done), (sample.files.len() as u32, sample.files.len() as u32, sample.total()));
    let (_, errors) = jfn_run::check_course(&format!("{}/beispiel", target));
    t.eq("  jfn-sim course finds no errors", errors, 0);
    for f in &sample.files {
        t.eq(&format!("  {}: the same bytes", f.path), plat::read_file(&format!("{}/beispiel/{}", target, f.path)).map(|b| sha(&b)), Some(f.sha256.clone()));
    }
    let entries = store::list(&target);
    t.check("  it can be offered", entries.iter().any(|e| e.id == "beispiel" && e.usable().is_some()));
    t.eq("  and uninstalled", said(&install::uninstall(&target, "beispiel")), None);
}

/// the group's checks
pub(crate) fn run(t: &mut T) {
    index_t(t);
    install_t(t);
    resume_t(t);
    update_t(t);
    uninstall_t(t);
    folder_t(t);
    words_t(t);
    relist_t(t);
    author_t(t);
    if let Some(root) = plat::arg_value("--academy-dir") {
        repo_t(t, root.replace('\\', "/").trim_end_matches('/'));
    }
}
