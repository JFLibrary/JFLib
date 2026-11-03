// install -- installing a course of the Akademie without any window: from the web or from a folder, for the
// app's worker thread and for jfn-sim (course-install, course-list, course-remove)
//
//   Source                        where the files come from: index() the bytes of index.json, fetch() one file
//                                 into its place; Web and Folder, a test brings its own
//   Web::new(base) -> Result<Web, Failure>   an http(s) address that ends in the course folders' parent;
//                                 update::fetch does the work, the app's net.connect rules decide
//   Folder::new(dir)              a folder laid out like the address: index.json, <kurs>/...
//   read_index(&src) -> Result<(Index, bytes), Failure> / parse_index(bytes)   the list and what it may ask for
//   Counters                      what a worker reports (seen(): phase, file n of m, bytes) and its cancel flag
//   install(&src, root, &course, &counters) -> Result<(), Failure>   the course in folder root/<id>
//   uninstall(root, id)           the course goes, whatever the learner did stays (it is not in its folder)
//   recover(root)                 after a crash: what a rename left behind (run before the courses are listed)
//   has_part(root, id) / forget_parts(root, &index)   a started installation: is there one, drop the ones of
//                                 courses the list no longer names
//   Failure { kind, msg, problems }   what went wrong: Kind for the words and what to do (the app's), msg the
//                                 exact message, problems what is wrong with a bad course
//   cli_install(args) / cli_list(args) / cli_remove(args)   jfn-sim's commands
//
// The way of a course, the same for every source:
//   1. its entry of the list is checked again: ids, paths (safe_path, safe_id), limits, checksums;
//   2. a folder root/.<id>.part is filled, only with what the list names; a file that is there and right stays,
//      a part of one goes on, what the list does not name is removed.  Every file is checked for its size and
//      SHA-256 here, whatever the source says;
//   3. the folder is checked as jfn-sim course checks it, under the course's id;
//   4. only then it is renamed into place.  An old version moves aside first (root/.<id>.old) and is removed
//      after: until the new one is complete nothing happens to it, and a failure leaves it as it was.
// A dot in front keeps these folders out of the list of courses (store.rs).  An interruption at any point leaves
// what was there usable: .part goes on at the next attempt, recover() settles .old and .trash.  A lock file in
// root keeps two programs from installing at once.
//
// Nothing of a course is executed here: its labs go through the simulator's checks, and the only things written
// are the files the list names, below root.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::SeqCst};

use jfn_model::academy::{FileEntry, Index, IndexCourse, COURSE_FILE, INDEX_MAX};
use jfn_model::msg;
use jfn_model::msg::Msg;
use netio::Url;
use plat::fs::FileKind;
use update::fetch::{self, Progress};

use crate::{complain, course_findings, say};

/// where the courses are by default: JFLib's repository; a repository of their own later is the same host
pub const DEFAULT_BASE: &str = "https://raw.githubusercontent.com/JFLibrary/JFLib/main/apps/JFNetworker/academy/";
/// the host apps/JFNetworker/PERMISSIONS asks about; anything else is refused without a question
pub const COURSE_HOST: &str = "raw.githubusercontent.com";
/// the list's name at a source
pub const INDEX_FILE: &str = "index.json";
/// the lock beside the courses
const LOCK_FILE: &str = ".install-lock";
/// how often a file goes on by itself while more of it arrives
const RETRIES: u32 = 3;
/// the pieces of a copy
const CHUNK: usize = 1 << 16;
/// what the program calls itself to a server
const AGENT: &str = "JFNetworker";
const AGENT_VERSION: &str = "0.1";
/// the helper thread of --stop-after
const WATCH_STACK: usize = 64 << 10;

/// what went wrong, for the words the app finds and what it tells to do
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// the address is none
    Address,
    /// no connection: no network, no name, nobody answers
    Offline,
    /// the connection broke off; what arrived stays for the next attempt
    Interrupted,
    /// the rule's question was refused, or nobody could be asked
    Denied,
    /// a host no rule names
    OtherHost,
    /// the certificate or the handshake was refused
    Tls,
    /// the server or the folder does not have it
    Missing,
    /// the list is no list, or asks for what it may not
    BadIndex,
    /// a file is not what the list says
    Checksum,
    /// more than the limits allow
    TooLarge,
    /// the files came, the course is not in order
    BadCourse,
    /// files could not be written
    Disk,
    /// another installation runs in this folder
    Busy,
    Cancelled,
}

impl Kind {
    /// its word on jfn-sim's command line
    pub fn word(self) -> &'static str {
        match self {
            Kind::Address => "address",
            Kind::Offline => "offline",
            Kind::Interrupted => "interrupted",
            Kind::Denied => "denied",
            Kind::OtherHost => "other-host",
            Kind::Tls => "tls",
            Kind::Missing => "missing",
            Kind::BadIndex => "bad-index",
            Kind::Checksum => "checksum",
            Kind::TooLarge => "too-large",
            Kind::BadCourse => "bad-course",
            Kind::Disk => "disk",
            Kind::Busy => "busy",
            Kind::Cancelled => "cancelled",
        }
    }
}

/// why a step failed
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: Kind,
    pub msg: Msg,
    /// a bad course: what is wrong with it
    pub problems: Vec<Msg>,
}

fn fail(kind: Kind, msg: Msg) -> Failure {
    Failure { kind, msg, problems: Vec::new() }
}

fn cancelled() -> Failure {
    fail(Kind::Cancelled, msg!("Abgebrochen."))
}

fn no_write(path: &str) -> Failure {
    fail(Kind::Disk, msg!("{} lässt sich nicht schreiben.", path))
}

fn wrong_size(path: &str, got: u64, want: u64) -> Failure {
    fail(Kind::Checksum, msg!("Die Datei {} hat {} Bytes; die Kursliste nennt {} Bytes.", path, got, want))
}

fn wrong_hash(path: &str) -> Failure {
    fail(Kind::Checksum, msg!("Die Prüfsumme der Datei {} stimmt nicht mit der Kursliste überein.", path))
}

/// what a worker is doing
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    /// the files come
    #[default]
    Fetching,
    /// the course is checked
    Checking,
    /// it goes into place
    Placing,
}

/// what the counters say
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Seen {
    pub phase: Phase,
    /// the file being fetched (from 1), of how many
    pub file: u32,
    pub files: u32,
    /// bytes of this course that are here, of how many
    pub done: u64,
    pub total: u64,
}

/// the counters a worker updates and the flag that stops it
#[derive(Debug, Default)]
pub struct Counters {
    pub cancel: AtomicBool,
    phase: AtomicU32,
    file: AtomicU32,
    files: AtomicU32,
    total: AtomicU64,
    /// the bytes of the files that are finished, of the file under way and its total (update::fetch's)
    finished: AtomicU64,
    file_done: AtomicU64,
    file_total: AtomicU64,
}

impl Counters {
    /// stop as soon as it can
    pub fn stop(&self) {
        self.cancel.store(true, SeqCst);
    }

    pub fn stopped(&self) -> bool {
        self.cancel.load(SeqCst)
    }

    pub fn seen(&self) -> Seen {
        let phase = match self.phase.load(SeqCst) {
            1 => Phase::Checking,
            2 => Phase::Placing,
            _ => Phase::Fetching,
        };
        Seen { phase, file: self.file.load(SeqCst), files: self.files.load(SeqCst), done: self.finished.load(SeqCst).saturating_add(self.file_done.load(SeqCst)), total: self.total.load(SeqCst) }
    }

    /// a source reports the bytes of the file under way that are here
    pub fn file_bytes(&self, done: u64) {
        self.file_done.store(done, SeqCst);
    }

    fn begin(&self, c: &IndexCourse) {
        self.phase.store(0, SeqCst);
        self.file.store(0, SeqCst);
        self.files.store(c.files.len() as u32, SeqCst);
        self.total.store(c.total(), SeqCst);
        self.finished.store(0, SeqCst);
        self.file_done.store(0, SeqCst);
    }
}

/// where the files of courses come from
pub trait Source {
    /// the bytes of index.json (more than INDEX_MAX: Kind::TooLarge)
    fn index(&self) -> Result<Vec<u8>, Failure>;
    /// file `f` of course `id` into the path `part`: what is there already is its start and goes on; reports
    /// through Counters::file_bytes, stops at Counters::stopped (Kind::Cancelled), Kind::Interrupted when the way
    /// broke off and what arrived is in `part`.  install() checks the result, whatever this says.
    fn fetch(&self, id: &str, f: &FileEntry, part: &str, c: &Counters) -> Result<(), Failure>;
}

// ---------------------------------------------------------------- the two sources

/// a server: <base>/index.json, <base>/<kurs>/<pfad>
pub struct Web {
    base: Url,
    cfg: update::Config,
}

// a failure of update::fetch, for the file it was after
fn from_update(what: &str, e: update::Error) -> Failure {
    use update::Error as E;
    match e {
        E::Denied(h) if h == COURSE_HOST => fail(Kind::Denied, msg!("Die Verbindung zu {} wurde nicht erlaubt.", h)),
        E::Denied(h) => fail(Kind::OtherHost, msg!("Der Host {} ist nicht erlaubt; Kurse kommen nur von {}.", h, COURSE_HOST)),
        E::Tls(s) => fail(Kind::Tls, msg!("Die sichere Verbindung kam nicht zustande: {}", Msg::plain(s.as_str()))),
        E::TooLarge(_) => fail(Kind::TooLarge, msg!("Die Kursliste ist größer als die erlaubten {} Bytes.", INDEX_MAX)),
        E::Net(s) => fail(Kind::Offline, msg!("Keine Verbindung: {}", Msg::plain(s.as_str()))),
        E::Http(code) => fail(Kind::Missing, msg!("Der Server antwortet für {} mit dem Status {}.", what, code)),
        E::Size { expected, got } => wrong_size(what, got, expected),
        E::Hash => wrong_hash(what),
        E::Incomplete { done, total } => fail(Kind::Interrupted, msg!("Die Verbindung brach bei {} von {} Bytes ab.", done, total)),
        E::Cancelled => cancelled(),
        E::Io(s) => fail(Kind::Disk, Msg::plain(s)),
        other => fail(Kind::Offline, msg!("Keine Verbindung: {}", Msg::plain(format!("{}", other)))),
    }
}

impl Web {
    /// the address of the folder that holds index.json and the courses
    pub fn new(base: &str) -> Result<Web, Failure> {
        let base = base.trim();
        let with_slash = if base.ends_with('/') { String::from(base) } else { format!("{}/", base) };
        let url = Url::parse(&with_slash).filter(|u| !u.host.is_empty()).ok_or_else(|| fail(Kind::Address, msg!("„{}“ ist keine Adresse für Kurse (http:// oder https://).", base)))?;
        Ok(Web { base: url, cfg: update::Config::new(AGENT, AGENT_VERSION, "", &[]) })
    }

    fn at(&self, rel: &str) -> Result<Url, Failure> {
        fetch::join(&self.base, rel).ok_or_else(|| fail(Kind::Address, msg!("„{}“ ist keine Adresse für Kurse (http:// oder https://).", rel)))
    }
}

impl Source for Web {
    fn index(&self) -> Result<Vec<u8>, Failure> {
        let u = self.at(INDEX_FILE)?;
        fetch::get(&self.cfg, &u, INDEX_MAX).map(|(_, b)| b).map_err(|e| from_update(INDEX_FILE, e))
    }

    fn fetch(&self, id: &str, f: &FileEntry, part: &str, c: &Counters) -> Result<(), Failure> {
        let u = self.at(&format!("{}/{}", id, f.path))?;
        let sha: [u8; 32] = crypto::unhex(&f.sha256).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()).ok_or_else(|| wrong_hash(&f.path))?;
        let p = Progress { done: &c.file_done, total: &c.file_total, cancel: &c.cancel };
        fetch::download(&self.cfg, &u, part, f.size, &sha, &p).map_err(|e| from_update(&f.path, e))
    }
}

/// a folder laid out like a server's address
pub struct Folder {
    root: String,
}

impl Folder {
    pub fn new(dir: &str) -> Folder {
        Folder { root: String::from(dir.replace('\\', "/").trim_end_matches('/')) }
    }
}

impl Source for Folder {
    fn index(&self) -> Result<Vec<u8>, Failure> {
        let path = format!("{}/{}", self.root, INDEX_FILE);
        match plat::fs::stat(&path) {
            None => return Err(fail(Kind::BadIndex, msg!("Im Ordner {} liegt keine {}. Autoren erzeugen sie mit tools/academy_index.py.", self.root, INDEX_FILE))),
            Some(m) if m.size > INDEX_MAX as u64 => return Err(fail(Kind::TooLarge, msg!("Die Kursliste ist größer als die erlaubten {} Bytes.", INDEX_MAX))),
            Some(_) => {}
        }
        plat::read_file(&path).ok_or_else(|| fail(Kind::Missing, msg!("Die Datei {} fehlt in der Quelle.", INDEX_FILE)))
    }

    fn fetch(&self, id: &str, f: &FileEntry, part: &str, c: &Counters) -> Result<(), Failure> {
        let from = format!("{}/{}/{}", self.root, id, f.path);
        // a link leads somewhere else: not a file of this course
        let m = plat::fs::lstat(&from).filter(|m| m.is_file()).ok_or_else(|| fail(Kind::Missing, msg!("Die Datei {} fehlt in der Quelle.", f.path)))?;
        if m.size != f.size {
            return Err(wrong_size(&f.path, m.size, f.size));
        }
        let src = plat::File::open(&from, plat::FO_READ).ok_or_else(|| fail(Kind::Missing, msg!("Die Datei {} fehlt in der Quelle.", f.path)))?;
        let dst = plat::File::open(part, plat::FO_RW).ok_or_else(|| no_write(part))?;
        let mut at = dst.size().unwrap_or(0);
        if at > f.size {
            dst.set_len(0);
            at = 0;
        }
        c.file_bytes(at);
        let mut buf = alloc::vec![0u8; CHUNK];
        while at < f.size {
            if c.stopped() {
                dst.sync();
                return Err(cancelled());
            }
            let want = (f.size - at).min(CHUNK as u64) as usize;
            let n = src.read_at(at, &mut buf[..want]).filter(|&n| n > 0).ok_or_else(|| fail(Kind::Interrupted, msg!("Die Datei {} fehlt in der Quelle.", f.path)))?;
            if !write_all(&dst, at, &buf[..n]) {
                return Err(no_write(part));
            }
            at += n as u64;
            c.file_bytes(at);
        }
        dst.sync();
        Ok(())
    }
}

fn write_all(f: &plat::File, mut off: u64, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        match f.write_at(off, data) {
            Some(n) if n > 0 => {
                off += n as u64;
                data = &data[n..];
            }
            _ => return false,
        }
    }
    true
}

/// a source by what the command line or the file dialog gives: an address, else a folder
pub fn source_of(s: &str) -> Result<Box<dyn Source>, Failure> {
    if s.contains("://") && !s.starts_with("mem://") {
        Ok(Box::new(Web::new(s)?))
    } else {
        Ok(Box::new(Folder::new(s)))
    }
}

// ---------------------------------------------------------------- the list

/// the bytes of index.json -> the list, or why not (its format and version, then Index::issues)
pub fn parse_index(bytes: &[u8]) -> Result<Index, Failure> {
    if bytes.len() > INDEX_MAX {
        return Err(fail(Kind::TooLarge, msg!("Die Kursliste ist größer als die erlaubten {} Bytes.", INDEX_MAX)));
    }
    let text = core::str::from_utf8(bytes).map_err(|_| fail(Kind::BadIndex, msg!("Die Kursliste ist kein UTF-8-Text.")))?;
    let index = Index::from_text(text).map_err(|m| fail(Kind::BadIndex, m))?;
    let bad: Vec<Msg> = index.issues().into_iter().filter(|p| p.fatal).map(|p| p.msg).collect();
    if let Some(first) = bad.first().cloned() {
        return Err(Failure { kind: limit_or_bad(&index), msg: first, problems: bad });
    }
    Ok(index)
}

// too large is a kind of its own: the message of Index::issues says by how much
fn limit_or_bad(index: &Index) -> Kind {
    use jfn_model::academy::{COURSE_BYTES_MAX, COURSE_FILES_MAX, FILE_MAX, INDEX_COURSES_MAX};
    let over = index.courses.len() > INDEX_COURSES_MAX || index.courses.iter().any(|c| c.files.len() > COURSE_FILES_MAX || c.total() > COURSE_BYTES_MAX || c.files.iter().any(|f| f.size > FILE_MAX));
    if over { Kind::TooLarge } else { Kind::BadIndex }
}

/// the list of a source: what it says and its bytes (the app keeps them)
pub fn read_index(src: &dyn Source) -> Result<(Index, Vec<u8>), Failure> {
    let bytes = src.index()?;
    let index = parse_index(&bytes)?;
    Ok((index, bytes))
}

// ---------------------------------------------------------------- the files

fn part_of(root: &str, id: &str) -> String {
    format!("{}/.{}.part", root, id)
}

fn old_of(root: &str, id: &str) -> String {
    format!("{}/.{}.old", root, id)
}

fn trash_of(root: &str, id: &str) -> String {
    format!("{}/.{}.trash", root, id)
}

fn size_of(path: &str) -> u64 {
    plat::fs::stat(path).map_or(0, |m| m.size)
}

// a folder with everything below it, links as links -> nothing is left of it
fn remove_tree(path: &str) -> bool {
    let Some(m) = plat::fs::lstat(path) else { return true };
    if !m.is_dir() {
        return plat::delete(path);
    }
    let mut ok = true;
    for e in plat::fs::read_dir(path).unwrap_or_default() {
        let below = format!("{}/{}", path, e.name);
        ok &= if e.kind == FileKind::Dir { remove_tree(&below) } else { plat::delete(&below) };
    }
    ok && plat::delete(path)
}

// what a part folder holds that the list does not name goes
fn prune(dir: &str, rel: &str, c: &IndexCourse) {
    for e in plat::fs::read_dir(dir).unwrap_or_default() {
        let name = if rel.is_empty() { e.name.clone() } else { format!("{}/{}", rel, e.name) };
        let path = format!("{}/{}", dir, e.name);
        if e.kind == FileKind::Dir {
            prune(&path, &name, c);
            if plat::fs::read_dir(&path).is_some_and(|v| v.is_empty()) {
                plat::delete(&path);
            }
        } else if !c.files.iter().any(|f| f.path == name) {
            plat::delete(&path);
        }
    }
}

// the SHA-256 of a file in lower-case hex
fn digest(path: &str) -> Option<String> {
    let f = plat::File::open(path, plat::FO_READ)?;
    let mut h = crypto::Sha256::new();
    let mut buf = alloc::vec![0u8; CHUNK];
    let mut off = 0u64;
    loop {
        let n = f.read_at(off, &mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        off += n as u64;
    }
    Some(crypto::hex(&h.finalize()))
}

// the file at `path` is exactly what the list says; a wrong one goes
fn check_file(path: &str, f: &FileEntry) -> Result<(), Failure> {
    let got = size_of(path);
    if got != f.size {
        plat::delete(path);
        return Err(wrong_size(&f.path, got, f.size));
    }
    if digest(path).as_deref() != Some(f.sha256.as_str()) {
        plat::delete(path);
        return Err(wrong_hash(&f.path));
    }
    Ok(())
}

// one file: what is there and right stays, what arrived goes on
fn fetch_file(src: &dyn Source, id: &str, f: &FileEntry, path: &str, c: &Counters) -> Result<(), Failure> {
    if plat::file_exists(path) == 1 && size_of(path) == f.size && digest(path).as_deref() == Some(f.sha256.as_str()) {
        return Ok(());
    }
    let (mut tries, mut fresh) = (0, false);
    loop {
        if c.stopped() {
            return Err(cancelled());
        }
        let before = size_of(path);
        c.file_bytes(before.min(f.size));
        match src.fetch(id, f, path, c).and_then(|()| check_file(path, f)) {
            Ok(()) => return Ok(()),
            // what arrived stays: while more arrives it goes on by itself
            Err(e) if e.kind == Kind::Interrupted && tries < RETRIES && size_of(path) > before => tries += 1,
            // the start came from an earlier attempt and belongs to something else: once from nothing
            Err(e) if e.kind == Kind::Checksum && before > 0 && !fresh => {
                plat::delete(path);
                fresh = true;
            }
            Err(e) => return Err(e),
        }
    }
}

// the part folder as jfn-sim course sees it, under the course's id
fn check_part(part: &str, c: &IndexCourse) -> Result<(), Failure> {
    let (found, labs) = course_findings(part, &c.id);
    let mut bad: Vec<Msg> = found.into_iter().filter(|p| p.fatal).map(|p| p.msg).collect();
    for (file, lines, errors) in labs {
        if errors > 0 {
            bad.extend(lines.iter().filter(|l| l.starts_with("Fehler")).map(|l| Msg::plain(format!("{}: {}", file, l.trim_end()))));
        }
    }
    if !bad.is_empty() {
        let n = bad.len();
        return Err(Failure { kind: Kind::BadCourse, msg: msg!("Der Kurs {} hat {} Fehler.", c.id, n), problems: bad });
    }
    let text = plat::read_file(&format!("{}/{}", part, COURSE_FILE)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    match jfn_model::academy::Course::from_text(&text) {
        Ok(x) if x.course_version != c.version => Err(fail(Kind::BadCourse, msg!("Die Kursdatei nennt die Version {}, die Kursliste die Version {}.", x.course_version, c.version))),
        Ok(_) => Ok(()),
        Err(m) => Err(fail(Kind::BadCourse, m)),
    }
}

// the checked part folder into place: an old version aside, the new one in, the old one gone
fn place(root: &str, id: &str) -> Result<(), Failure> {
    let (dir, part, old) = (format!("{}/{}", root, id), part_of(root, id), old_of(root, id));
    let there = plat::file_exists(&dir) != 0;
    if there {
        remove_tree(&old);
        if !plat::rename(&dir, &old) {
            return Err(no_write(&dir));
        }
    }
    if !plat::rename(&part, &dir) {
        if there {
            plat::rename(&old, &dir);
        }
        return Err(no_write(&dir));
    }
    // if this fails or never happens, recover() removes it
    remove_tree(&old);
    plat::fs::sync_dir(root);
    Ok(())
}

// the lock of this folder: held by someone, or a folder that cannot be written
fn lock(root: &str) -> Result<plat::FileLock, Failure> {
    let path = format!("{}/{}", root, LOCK_FILE);
    plat::FileLock::acquire(&path).ok_or_else(|| if plat::file_exists(&path) == 1 { fail(Kind::Busy, msg!("Eine andere Installation läuft gerade in diesem Ordner.")) } else { no_write(root) })
}

/// the course into folder root/<id> (the way at the top of this file); an old version stays until this is complete
pub fn install(src: &dyn Source, root: &str, course: &IndexCourse, c: &Counters) -> Result<(), Failure> {
    let one = Index { format: String::from(jfn_model::academy::INDEX_FORMAT), version: jfn_model::academy::VERSION, courses: alloc::vec![course.clone()] };
    let bad: Vec<Msg> = one.issues().into_iter().filter(|p| p.fatal).map(|p| p.msg).collect();
    if let Some(first) = bad.first().cloned() {
        return Err(Failure { kind: limit_or_bad(&one), msg: first, problems: bad });
    }
    if !plat::mkdir(root) {
        return Err(no_write(root));
    }
    let _lock = lock(root)?;
    recover(root);
    c.begin(course);
    let part = part_of(root, &course.id);
    if !plat::mkdir(&part) {
        return Err(no_write(&part));
    }
    prune(&part, "", course);
    for (k, f) in course.files.iter().enumerate() {
        c.file.store(k as u32 + 1, SeqCst);
        c.file_bytes(0);
        let path = format!("{}/{}", part, f.path);
        if let Some((dir, _)) = path.rsplit_once('/')
            && !plat::mkdir(dir)
        {
            return Err(no_write(dir));
        }
        fetch_file(src, &course.id, f, &path, c)?;
        c.file_bytes(0);
        c.finished.fetch_add(f.size, SeqCst);
    }
    if c.stopped() {
        return Err(cancelled());
    }
    c.phase.store(1, SeqCst);
    if let Err(e) = check_part(&part, course) {
        // the files are as the list says: only a new list can help, nothing of this goes on
        remove_tree(&part);
        return Err(e);
    }
    if c.stopped() {
        return Err(cancelled());
    }
    c.phase.store(2, SeqCst);
    place(root, &course.id)
}

/// the course goes: first its name, so that a crash leaves no half course.  What the learner did is kept by
/// id in progress.json and stays.
pub fn uninstall(root: &str, id: &str) -> Result<(), Failure> {
    if !jfn_model::academy::safe_id(id) {
        return Err(fail(Kind::BadCourse, msg!("Die Kennung „{}“ darf nur aus kleinen Buchstaben, Ziffern, - und _ bestehen.", id)));
    }
    let _lock = lock(root)?;
    let (dir, trash) = (format!("{}/{}", root, id), trash_of(root, id));
    remove_tree(&trash);
    if plat::file_exists(&dir) != 0 && !plat::rename(&dir, &trash) {
        return Err(no_write(&dir));
    }
    remove_tree(&trash);
    remove_tree(&part_of(root, id));
    remove_tree(&old_of(root, id));
    Ok(())
}

// the id in a leftover's name: ".<id>.<end>"
fn left_id<'a>(name: &'a str, end: &str) -> Option<&'a str> {
    name.strip_prefix('.')?.strip_suffix(end)?.strip_suffix('.').filter(|id| jfn_model::academy::safe_id(id))
}

/// what an interrupted change left: an old version whose new one never arrived comes back, an old version beside
/// its new one goes, and so does what was on its way out.  A .part stays: the next attempt goes on with it.
pub fn recover(root: &str) {
    for e in plat::fs::read_dir(root).unwrap_or_default() {
        if e.kind != FileKind::Dir {
            continue;
        }
        let path = format!("{}/{}", root, e.name);
        if let Some(id) = left_id(&e.name, "old") {
            let dir = format!("{}/{}", root, id);
            if plat::file_exists(&dir) == 0 {
                plat::rename(&path, &dir);
            } else {
                remove_tree(&path);
            }
        } else if left_id(&e.name, "trash").is_some() {
            remove_tree(&path);
        }
    }
}

/// an installation of this course was started and not finished
pub fn has_part(root: &str, id: &str) -> bool {
    plat::file_exists(&part_of(root, id)) == 2
}

/// the started installations of courses the list does not name any more go
pub fn forget_parts(root: &str, index: &Index) {
    for e in plat::fs::read_dir(root).unwrap_or_default() {
        if let Some(id) = left_id(&e.name, "part")
            && !index.courses.iter().any(|c| c.id == id)
        {
            remove_tree(&format!("{}/{}", root, e.name));
        }
    }
}

// ---------------------------------------------------------------- jfn-sim

const USAGE: &str = "jfn-sim course-install QUELLE KURS|all --into ORDNER [--index DATEI] [--stop-after BYTES]\njfn-sim course-list QUELLE [--into ORDNER] [--index DATEI]\njfn-sim course-remove KURS --into ORDNER\n";

// the values of the options and the rest
fn cli_args(args: &[String]) -> Result<(Vec<String>, Option<String>, Option<String>, Option<u64>), String> {
    let (mut rest, mut into, mut index, mut stop) = (Vec::new(), None, None, None);
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--into" | "--index" | "--stop-after" => {
                i += 1;
                let v = args.get(i).cloned().ok_or_else(|| format!("{} braucht einen Wert.", a))?;
                match a {
                    "--into" => into = Some(String::from(v.replace('\\', "/").trim_end_matches('/'))),
                    "--index" => index = Some(v),
                    _ => stop = Some(v.parse::<u64>().map_err(|_| format!("--stop-after braucht eine Zahl, nicht {}.", v))?),
                }
            }
            a if a.starts_with("--") => return Err(format!("Die Option {} gibt es nicht.", a)),
            _ => rest.push(String::from(a)),
        }
        i += 1;
    }
    Ok((rest, into, index, stop))
}

// a failure on stderr: its words, its kind for scripts; 3: cancelled, 1: anything else
fn report(who: &str, e: &Failure) -> i32 {
    complain(&format!("{}: {} [{}]\n", who, e.msg.text(), e.kind.word()));
    for p in &e.problems {
        complain(&format!("  {}\n", p.text()));
    }
    if e.kind == Kind::Cancelled { 3 } else { 1 }
}

// the list of the source, or of a file named instead (a source that has no list yet)
fn list_of(src: &dyn Source, file: Option<&str>) -> Result<Index, Failure> {
    match file {
        Some(f) => parse_index(&plat::read_file(f).ok_or_else(|| fail(Kind::BadIndex, msg!("Die Datei {} fehlt in der Quelle.", f)))?),
        None => read_index(src).map(|x| x.0),
    }
}

/// jfn-sim course-install SOURCE COURSE|all --into DIR [--index FILE] [--stop-after BYTES]: an address or a
/// folder; --stop-after cancels when that many bytes of a course are here (what a cancel leaves is resumed by the
/// next run).  Exit codes: 0 installed, 1 a failure (on stderr with its kind), 2 the command line, 3 cancelled.
pub fn cli_install(args: &[String]) -> i32 {
    let (rest, into, index, stop) = match cli_args(args) {
        Ok(x) => x,
        Err(e) => {
            complain(&format!("{}\n{}", e, USAGE));
            return 2;
        }
    };
    let ([source, course], Some(into)) = (rest.as_slice(), into) else {
        complain(USAGE);
        return 2;
    };
    // the command line grants what it names as it is at the start: only a folder that exists with everything below it
    if plat::file_exists(&into) != 2 {
        complain(&format!("Den Ordner {} gibt es nicht; legen Sie ihn zuerst an.\n", into));
        return 2;
    }
    let src = match source_of(source) {
        Ok(s) => s,
        Err(e) => return report(source, &e),
    };
    let list = match list_of(src.as_ref(), index.as_deref()) {
        Ok(l) => l,
        Err(e) => return report(source, &e),
    };
    let chosen: Vec<&IndexCourse> = list.courses.iter().filter(|c| course == "all" || c.id == *course).collect();
    if chosen.is_empty() {
        complain(&format!("{}: die Kursliste nennt keinen Kurs {}.\n", source, course));
        return 1;
    }
    let mut code = 0;
    for c in chosen {
        let counters = Arc::new(Counters::default());
        let watch = stop.and_then(|n| {
            let (w, done) = (counters.clone(), Arc::new(AtomicBool::new(false)));
            let d = done.clone();
            plat::thread_spawn(
                WATCH_STACK,
                Box::new(move || {
                    while !d.load(SeqCst) && w.seen().done < n {
                        plat::thread_yield();
                    }
                    if !d.load(SeqCst) {
                        w.stop();
                    }
                }),
            )
            .map(|t| (t, done))
        });
        let r = install(src.as_ref(), &into, c, &counters);
        if let Some((t, done)) = watch {
            done.store(true, SeqCst);
            t.join();
        }
        match r {
            Ok(()) => say(&format!("{}: Version {}, {} Dateien, {} Bytes: installiert\n", c.id, c.version, c.files.len(), c.total())),
            Err(e) => code = code.max(report(&c.id, &e)),
        }
    }
    code
}

/// jfn-sim course-list SOURCE [--into DIR] [--index FILE]: a line per course of the list: id, version, files,
/// bytes, a sample or not, with --into how it stands against that folder, its title
pub fn cli_list(args: &[String]) -> i32 {
    use jfn_model::academy::{standing, Standing};
    let (rest, into, index, _) = match cli_args(args) {
        Ok(x) => x,
        Err(e) => {
            complain(&format!("{}\n{}", e, USAGE));
            return 2;
        }
    };
    let [source] = rest.as_slice() else {
        complain(USAGE);
        return 2;
    };
    let src = match source_of(source) {
        Ok(s) => s,
        Err(e) => return report(source, &e),
    };
    let list = match list_of(src.as_ref(), index.as_deref()) {
        Ok(l) => l,
        Err(e) => return report(source, &e),
    };
    for c in &list.courses {
        let state = into.as_deref().map(|d| {
            let here = plat::read_file(&format!("{}/{}/{}", d, c.id, COURSE_FILE)).and_then(|b| jfn_model::academy::Course::from_text(&String::from_utf8_lossy(&b)).ok());
            match standing(here.map(|x| x.course_version), Some(c.version)) {
                Standing::NotInstalled => " nicht installiert",
                Standing::Current | Standing::Unlisted => " aktuell",
                Standing::Update => " neuere Version",
                Standing::Newer => " hier neuer",
            }
        });
        say(&format!("{} Version {}, {} Dateien, {} Bytes{}{}: {}\n", c.id, c.version, c.files.len(), c.total(), if c.sample { ", Beispiel" } else { "" }, state.unwrap_or(""), c.title.de));
    }
    0
}

/// jfn-sim course-remove COURSE --into DIR
pub fn cli_remove(args: &[String]) -> i32 {
    let (rest, into, _, _) = match cli_args(args) {
        Ok(x) => x,
        Err(e) => {
            complain(&format!("{}\n{}", e, USAGE));
            return 2;
        }
    };
    let ([course], Some(into)) = (rest.as_slice(), into) else {
        complain(USAGE);
        return 2;
    };
    match uninstall(&into, course) {
        Ok(()) => {
            say(&format!("{}: deinstalliert\n", course));
            0
        }
        Err(e) => report(course, &e),
    }
}
