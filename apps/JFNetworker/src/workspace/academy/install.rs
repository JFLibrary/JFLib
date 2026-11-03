// install -- how courses come to the Akademie, on its page: what can be installed, what is installed, a worker
// that does it, what it says when it cannot.  The way of a course itself is jfn_run::install.
//
//   Inst::new()               nothing read yet
//   read_last(a)              Academy::ensure: settles what a crash left, the list that was kept
//   tick(n, ui)               every frame: a worker that ended, the courses listed again, its timer
//   timer(n, ui)              TIMER_AI_JOB: a frame, so that the progress moves
//   pending() -> bool         the sync hook of scripts: a worker runs and is not waiting at the gate
//   command(n, ui, id, arg)   CMD_AI_SEARCH .. CMD_AI_GATE
//   tools(n, ui)              on the list of courses: Nach Kursen suchen, Aus Ordner installieren, what a worker does
//   empty(n, ui)              what the empty list offers: Jetzt installieren
//   row(n, ui, i)             under a course's card: version, size, state, Aktualisieren, Deinstallieren
//   offered(n, ui)            what the list offers that is not here
//   dialog(n, ui)             "Kurs deinstallieren?", "Diese Adresse öffnen?"
//   ask_link(n, ui, url)      a lesson's web link: the question that shows the address
//   standing_of(n, e)         a course against the list
//
// No network without a click: nothing here runs at the start, and the first connection is a worker's, so
// that the question of `ask net.connect` in PERMISSIONS comes with that click and the window keeps running while it
// waits.  The list that was read last is kept (data://academy/last-index.json, read only here), so the page knows
// it offline; the courses themselves never need the network.  The address is a setting (prefs.rs, Einstellungen ›
// Akademie, with its way back); another host than the rule's is refused without a question and said so.
//
// A worker is a thread of its own with a large stack (a course's labs are checked with the simulator), one at a
// time; it never touches Net.  Going to another page leaves it running, leaving the program leaves what jfn_run's way
// can always pick up.  After a change the courses are listed again (Academy::relist) unless a quiz's page shows
// or a test runs: those live on numbers of the list and on what the learner typed, so the list waits for them.
//
// For scripts: --academy-gate makes a run without a window stop its worker before each file until `cmd 1039 N` lets N
// files through; `sync` counts it as idle while it stands there, so a script sees "Datei 2 von 3" for certain.
// --academy-folder PATH is where the folder dialog starts.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

use jfn_model::academy::{standing, FileEntry, Index, IndexCourse, Standing};
use jfn_model::lab::Text;
use jfn_run::install::{self as way, Counters, Failure, Folder, Kind, Phase, Source, Web, COURSE_HOST};
use plat::sync::Mutex;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::workspace::academy::store::Entry;
use crate::workspace::academy::{level_text, Academy, View};
use crate::workspace::task::mark;

/// the reason of the `ask net.connect` line in PERMISSIONS: the question shows it through the catalogue
pub(crate) const ASK_REASON: &str = "Kurse der Akademie laden"; // i18n
/// the worker's stack, as a sweep's
const STACK: usize = 4 << 20;
/// the file beside the courses with the list that was read last
const LAST_INDEX: &str = "last-index.json";
/// how often the progress moves (ms)
const TICK_MS: u32 = 100;

/// workers running
static BUSY: AtomicU32 = AtomicU32::new(0);
/// the scripts' gate: it stops workers, files it lets through, a worker stands at it, cancel was asked for
static GATE_ON: AtomicBool = AtomicBool::new(false);
static GATE_ALLOW: AtomicU32 = AtomicU32::new(0);
static GATE_AT: AtomicBool = AtomicBool::new(false);
static GATE_STOP: AtomicBool = AtomicBool::new(false);

/// a worker runs and is not waiting at the gate (ui.sync_hook_set)
pub(crate) fn pending() -> bool {
    BUSY.load(SeqCst) > 0 && (GATE_STOP.load(SeqCst) || GATE_ALLOW.load(SeqCst) > 0 || !GATE_AT.load(SeqCst))
}

/// what a worker is for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum What {
    Search,
    Folder,
    Install,
    Remove,
}

/// where a worker's files come from
#[derive(Clone, Debug, PartialEq, Eq)]
enum Spec {
    Web(String),
    Folder(String),
}

/// what a worker is told to do
enum Work {
    /// read the list; `keep`: write it there
    Read { from: Spec, root: String, keep: Option<String> },
    /// read the list, then what a first install takes
    All { from: Spec, root: String, keep: Option<String>, here: Vec<(String, u32)> },
    Courses { from: Spec, root: String, list: Vec<IndexCourse> },
    Remove { root: String, id: String },
}

/// what a worker found
#[derive(Default)]
struct Done {
    index: Option<Index>,
    /// the courses that changed, what they are called
    changed: Vec<String>,
    titles: Vec<Text>,
    failure: Option<Failure>,
    /// everything was here already
    nothing: bool,
}

/// what a worker and the page share
struct State {
    c: Counters,
    /// the course under way (from 1) of how many, its title
    course: AtomicU32,
    courses: AtomicU32,
    title: Mutex<Text>,
    finished: AtomicBool,
    done: Mutex<Option<Done>>,
}

/// a worker
struct Job {
    what: What,
    from_folder: Option<String>,
    st: Arc<State>,
    thread: Option<plat::Thread>,
}

/// what the page has of installing
pub(crate) struct Inst {
    /// the list of the server (the last one read), of a folder (over it while it is open)
    pub server: Option<Index>,
    pub folder: Option<(String, Index)>,
    job: Option<Job>,
    armed: bool,
    /// the courses changed and are not listed again yet (quizzes first)
    relist: bool,
    changed: Vec<String>,
    /// the question before uninstalling: its course
    pub ask_remove: Shared<i32>,
    remove: Option<(String, Text)>,
    /// the question before a web link: its address
    pub ask_link: Shared<i32>,
    pub link: String,
    /// where the folder dialog starts
    pub start_folder: String,
    /// this run has the scripts' gate
    gate: bool,
    /// what the last worker has to say: its kind (IB_*), title and text, on the list of courses until it is closed
    note: Option<(u32, String, String)>,
}

impl Inst {
    pub(crate) fn new() -> Inst {
        Inst { server: None, folder: None, job: None, armed: false, relist: false, changed: Vec::new(), ask_remove: Shared::new(0), remove: None, ask_link: Shared::new(0), link: String::new(), start_folder: String::new(), gate: false, note: None }
    }

    /// Net::init: what a run without a window was given for its scripts
    pub(crate) fn setup(&mut self, headless: bool) {
        if headless {
            self.gate = plat::args().iter().any(|a| a == "--academy-gate");
            GATE_ON.store(self.gate, SeqCst);
            self.start_folder = plat::arg_value("--academy-folder").unwrap_or_default();
        }
    }

    /// the list the page shows
    fn offer(&self) -> Option<&Index> {
        self.folder.as_ref().map(|x| &x.1).or(self.server.as_ref())
    }

    /// a worker runs
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }
}

/// Academy::ensure: what a crash left is settled, the list that was kept is known again
pub(crate) fn read_last(a: &mut Academy) {
    let Some(root) = a.place.courses.clone() else { return };
    way::recover(&root);
    let text = plat::read_file(&format!("{}/{}", root, LAST_INDEX));
    a.inst.server = text.and_then(|b| way::parse_index(&b).ok());
}

// ---------------------------------------------------------------- the worker

// the scripts' gate in front of a source
struct Gated<'a>(&'a dyn Source);

impl Source for Gated<'_> {
    fn index(&self) -> Result<Vec<u8>, Failure> {
        self.0.index()
    }

    fn fetch(&self, id: &str, f: &FileEntry, part: &str, c: &Counters) -> Result<(), Failure> {
        while GATE_ON.load(SeqCst) && GATE_ALLOW.load(SeqCst) == 0 && !c.stopped() {
            GATE_AT.store(true, SeqCst);
            plat::sleep_ms(1);
        }
        GATE_AT.store(false, SeqCst);
        if GATE_ON.load(SeqCst) && !c.stopped() {
            GATE_ALLOW.fetch_sub(1, SeqCst);
        }
        self.0.fetch(id, f, part, c)
    }
}

fn source_of(from: &Spec) -> Result<Box<dyn Source>, Failure> {
    Ok(match from {
        Spec::Web(base) => Box::new(Web::new(base)?),
        Spec::Folder(dir) => Box::new(Folder::new(dir)),
    })
}

// the list of a source, kept if the page wants it kept; started installations of courses it no longer names go
fn read_list(src: &dyn Source, root: &str, keep: Option<&str>) -> Result<Index, Failure> {
    let (index, bytes) = way::read_index(src)?;
    if let Some(path) = keep {
        // a list that cannot be kept is no reason not to use it
        plat::mkdir(root);
        plat::write_file(path, &bytes);
        way::forget_parts(root, &index);
    }
    Ok(index)
}

// what a first install takes: the courses that are no samples; a list with nothing else: its samples
fn first_install(index: &Index, here: &[(String, u32)]) -> Vec<IndexCourse> {
    let real = index.courses.iter().any(|c| !c.sample);
    let wanted = |c: &&IndexCourse| c.sample != real;
    let stands = |c: &IndexCourse| standing(here.iter().find(|h| h.0 == c.id).map(|h| h.1), Some(c.version));
    index.courses.iter().filter(wanted).filter(|c| matches!(stands(c), Standing::NotInstalled | Standing::Update)).cloned().collect()
}

// course after course; the first failure ends it, what came before stands
fn take_courses(src: &dyn Source, root: &str, list: &[IndexCourse], st: &State, done: &mut Done) {
    st.courses.store(list.len() as u32, SeqCst);
    for (k, c) in list.iter().enumerate() {
        st.course.store(k as u32 + 1, SeqCst);
        *st.title.lock() = c.title.clone();
        match way::install(src, root, c, &st.c) {
            Ok(()) => {
                done.changed.push(c.id.clone());
                done.titles.push(c.title.clone());
            }
            Err(e) => {
                done.failure = Some(e);
                return;
            }
        }
    }
}

fn run(work: Work, st: &State) -> Done {
    let mut done = Done::default();
    match work {
        Work::Read { from, root, keep } => match source_of(&from).and_then(|s| read_list(s.as_ref(), &root, keep.as_deref())) {
            Ok(i) => done.index = Some(i),
            Err(e) => done.failure = Some(e),
        },
        Work::All { from, root, keep, here } => {
            let src = match source_of(&from) {
                Ok(s) => s,
                Err(e) => {
                    done.failure = Some(e);
                    return done;
                }
            };
            match read_list(src.as_ref(), &root, keep.as_deref()) {
                Ok(i) => {
                    let list = first_install(&i, &here);
                    done.index = Some(i);
                    done.nothing = list.is_empty();
                    take_courses(&Gated(src.as_ref()), &root, &list, st, &mut done);
                }
                Err(e) => done.failure = Some(e),
            }
        }
        Work::Courses { from, root, list } => match source_of(&from) {
            Ok(src) => take_courses(&Gated(src.as_ref()), &root, &list, st, &mut done),
            Err(e) => done.failure = Some(e),
        },
        Work::Remove { root, id } => match way::uninstall(&root, &id) {
            Ok(()) => done.changed.push(id),
            Err(e) => done.failure = Some(e),
        },
    }
    done
}

// a worker for this; one at a time, and the page shows it from now on
fn start(n: &mut Net, ui: &mut Ui, what: What, from_folder: Option<String>, work: Work) {
    let a = &mut n.academy;
    if a.inst.job.is_some() {
        return;
    }
    let st = Arc::new(State { c: Counters::default(), course: AtomicU32::new(0), courses: AtomicU32::new(0), title: Mutex::new(Text::default()), finished: AtomicBool::new(false), done: Mutex::new(None) });
    let s = st.clone();
    GATE_STOP.store(false, SeqCst);
    BUSY.fetch_add(1, SeqCst);
    let body = Box::new(move || {
        let done = run(work, &s);
        *s.done.lock() = Some(done);
        s.finished.store(true, SeqCst);
        BUSY.fetch_sub(1, SeqCst);
        plat::wake();
    });
    let Some(thread) = plat::thread_spawn(STACK, body) else {
        BUSY.fetch_sub(1, SeqCst);
        n.file.msg = Some((IB_ERROR, String::from(t("Das ließ sich nicht starten")), String::from(t("Es konnte kein weiterer Thread gestartet werden. Versuchen Sie es noch einmal."))));
        return;
    };
    a.inst.job = Some(Job { what, from_folder, st, thread: Some(thread) });
    a.inst.note = None;
    a.focus = ID_AI_CANCEL;
    n.file.msg = None;
    ui.repass();
}

// the words for what went wrong, and what to do about it
fn words(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Address => (t("Die Adresse der Kurse stimmt nicht"), t("Unter Einstellungen › Akademie lässt sie sich ändern und auf die Vorgabe zurücksetzen.")),
        Kind::Offline => (t("Keine Verbindung zum Kursserver"), t("Prüfen Sie die Internetverbindung und versuchen Sie es noch einmal. Installierte Kurse bleiben nutzbar. Einen HTTP-Proxy kann das Programm nicht nutzen; ohne Netz bleibt „Aus Ordner installieren“.")),
        Kind::Interrupted => (t("Die Übertragung brach ab"), t("Versuchen Sie es noch einmal. Was schon angekommen ist, bleibt, und es geht dort weiter.")),
        Kind::Denied => (t("Die Verbindung wurde nicht erlaubt"), t("Ohne Erlaubnis lädt das Programm keine Kurse. Beim nächsten Klick fragt es wieder; eine Antwort, die für immer gilt, lässt sich unter Einstellungen zurücksetzen. Ohne Netz bleibt „Aus Ordner installieren“.")),
        Kind::OtherHost => (t("Dieser Host ist nicht erlaubt"), t("Unter Einstellungen › Akademie lässt sich die Adresse auf die Vorgabe zurücksetzen.")),
        Kind::Tls => (t("Die sichere Verbindung kam nicht zustande"), t("Das Programm erkennt das Zertifikat des Servers nicht an. Prüfen Sie die Uhrzeit des Rechners. Ein Netz, das TLS aufbricht, geht nur, wenn seine Wurzel im System liegt.")),
        Kind::Missing => (t("Eine Datei fehlt an der Quelle"), t("Die Kursliste passt nicht zu den Dateien. Versuchen Sie es später noch einmal; was vorher da war, bleibt.")),
        Kind::BadIndex => (t("Die Kursliste ist nicht in Ordnung"), t("Installierte Kurse bleiben nutzbar. Autoren erzeugen die Liste mit tools/academy_index.py.")),
        Kind::Checksum => (t("Eine Datei passt nicht zur Kursliste"), t("Sie wurde verworfen, was vorher da war, bleibt. Versuchen Sie es noch einmal; bleibt es dabei, stimmen Liste und Dateien an der Quelle nicht überein.")),
        Kind::TooLarge => (t("Das ist zu groß"), t("Das Programm nimmt so große Kurse nicht an. Was vorher da war, bleibt.")),
        Kind::BadCourse => (t("Dieser Kurs ist nicht in Ordnung"), t("Er wurde nicht installiert, was vorher da war, bleibt. Das sagt die Prüfung:")),
        Kind::Disk => (t("Die Dateien ließen sich nicht schreiben"), t("Prüfen Sie den freien Platz und die Rechte des Ordners. Was vorher da war, bleibt.")),
        Kind::Busy => (t("Es läuft schon eine Installation"), t("Vielleicht in einem anderen Fenster des Programms. Warten Sie, bis sie fertig ist.")),
        Kind::Cancelled => (t("Abgebrochen"), t("Was schon angekommen ist, bleibt, und beim nächsten Mal geht es dort weiter. Was vorher da war, ist unverändert.")),
    }
}

/// a failure's kind, title and text: what happened, what to do, at most three of a bad course's problems,
/// each on a line of its own
pub(crate) fn failure_text(f: &Failure) -> (u32, String, String) {
    let (title, todo) = words(f.kind);
    let sev = if f.kind == Kind::Cancelled { IB_INFO } else { IB_ERROR };
    let mut text = if f.kind == Kind::Cancelled { String::from(todo) } else { format!("{}\n{}", tr::msg(&f.msg), todo) };
    for p in f.problems.iter().take(3) {
        text.push('\n');
        text.push_str(&tr::msg(p));
    }
    (sev, String::from(title), text)
}

// a worker ended: what it found on the page, what it says, the courses again
fn finish(n: &mut Net, ui: &mut Ui) {
    let Some(job) = n.academy.inst.job.take() else { return };
    if let Some(th) = job.thread {
        th.join();
    }
    let done = job.st.done.lock().take().unwrap_or_default();
    GATE_STOP.store(false, SeqCst);
    let a = &mut n.academy;
    let en = tr::en();
    let title_of = |d: &Done| d.titles.first().map_or(String::new(), |x| String::from(x.get(en)));
    let said = match (&done.failure, job.what) {
        (Some(f), _) => Some(failure_text(f)),
        (None, What::Search) => {
            let (all, new) = done.index.as_ref().map_or((0, 0), |i| (i.courses.len(), i.courses.iter().filter(|c| !a.entries.iter().any(|e| e.id == c.id)).count()));
            Some((IB_INFO, String::from(t("Die Kursliste ist geladen")), tf("Sie nennt {} Kurse, davon {} noch nicht installiert.", &[&all.to_string(), &new.to_string()])))
        }
        (None, What::Folder) => None,
        (None, What::Install) if done.nothing => Some((IB_INFO, String::from(t("Nichts zu installieren")), String::from(t("Alles aus der Kursliste ist schon da.")))),
        (None, What::Install) if done.titles.len() == 1 => Some((IB_SUCCESS, tf("„{}“ ist installiert", &[&title_of(&done)]), String::from(t("Der Kurs steht unter Meine Kurse.")))),
        (None, What::Install) => Some((IB_SUCCESS, tf("{} Kurse sind installiert", &[&done.titles.len().to_string()]), String::from(t("Sie stehen unter Meine Kurse.")))),
        (None, What::Remove) => {
            let name = a.inst.remove.as_ref().map_or(String::new(), |x| String::from(x.1.get(en)));
            Some((IB_INFO, tf("„{}“ ist deinstalliert", &[&name]), String::from(t("Ihr Fortschritt bleibt erhalten und gilt wieder, wenn Sie den Kurs erneut installieren."))))
        }
    };
    if let Some(i) = done.index {
        match job.from_folder {
            Some(dir) => a.inst.folder = Some((dir, i)),
            None => a.inst.server = Some(i),
        }
    }
    if !done.changed.is_empty() {
        a.inst.changed.extend(done.changed);
        a.inst.relist = true;
    }
    // on the list a note with room for all of it and the keyboard back (Abbrechen is gone); on any other page the info
    // bar too, or nobody would see it, and the keyboard stays where it is
    if a.on && a.view == View::Home {
        a.focus = ID_AI_SEARCH;
    } else {
        n.file.msg = said.as_ref().map(|(k, title, text)| (*k, title.clone(), text.replace('\n', " ")));
    }
    n.academy.inst.note = said;
    ui.repass();
}

// nothing depends on the old list: no quiz page shows, no test runs
fn list_free(a: &Academy) -> bool {
    !matches!(a.view, View::Quiz(_)) || !a.on
}

/// every frame: a worker that ended, the courses again as soon as nothing depends on the old list, the timer
pub(crate) fn tick(n: &mut Net, ui: &mut Ui) {
    if n.academy.inst.job.as_ref().is_some_and(|j| j.st.finished.load(SeqCst)) {
        finish(n, ui);
    }
    let a = &mut n.academy;
    if a.inst.relist && list_free(a) && !super::quiz::running(a) {
        a.inst.relist = false;
        let changed = core::mem::take(&mut a.inst.changed);
        let moved = a.relist(&changed);
        a.focus = match a.entries.iter().position(|e| changed.first().is_some_and(|c| *c == e.id)) {
            Some(i) if a.on && a.view == View::Home => ID_AK_CARD + i as u32,
            _ => a.focus,
        };
        if moved {
            n.file.msg = Some((IB_INFO, String::from(t("Dieser Kurs hat sich geändert")), String::from(t("Er wurde neu eingelesen; die Seite zeigt wieder den Anfang des Kurses. Ihr Fortschritt bleibt."))));
        }
        ui.repass();
    }
    let a = &mut n.academy;
    if a.inst.job.is_some() && !a.inst.armed {
        a.inst.armed = true;
        ui.timer(TIMER_AI_JOB, TICK_MS);
    }
}

/// TIMER_AI_JOB: a frame (its tick sets the timer again while a worker runs)
pub(crate) fn timer(n: &mut Net, ui: &mut Ui) {
    n.academy.inst.armed = false;
    ui.repass();
}

// ---------------------------------------------------------------- the commands

// where the courses are, what is here, where the list is kept
fn root_of(a: &Academy) -> Option<String> {
    a.place.courses.clone()
}

fn keep_path(root: &str) -> Option<String> {
    Some(format!("{}/{}", root, LAST_INDEX))
}

// the source of the list on the page
fn spec_of(n: &Net) -> Spec {
    match &n.academy.inst.folder {
        Some((dir, _)) => Spec::Folder(dir.clone()),
        None => Spec::Web(n.prefs.academy_base.clone()),
    }
}

// this course of the list on the page
fn take(n: &mut Net, ui: &mut Ui, c: IndexCourse) {
    let Some(root) = root_of(&n.academy) else { return };
    let from = spec_of(n);
    let folder = n.academy.inst.folder.as_ref().map(|x| x.0.clone());
    start(n, ui, What::Install, folder, Work::Courses { from, root, list: alloc::vec![c] });
}

/// the commands of installing
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_AI_SEARCH => {
            let Some(root) = root_of(&n.academy) else { return true };
            // back from a folder's list to the server's
            n.academy.inst.folder = None;
            let (from, keep) = (Spec::Web(n.prefs.academy_base.clone()), keep_path(&root));
            start(n, ui, What::Search, None, Work::Read { from, root, keep });
        }
        CMD_AI_ALL => {
            let Some(root) = root_of(&n.academy) else { return true };
            let here = n.academy.entries.iter().filter_map(|e| e.course.as_ref().map(|c| (e.id.clone(), c.course_version))).collect();
            let (from, keep) = (Spec::Web(n.prefs.academy_base.clone()), keep_path(&root));
            n.academy.inst.folder = None;
            start(n, ui, What::Install, None, Work::All { from, root, keep, here });
        }
        CMD_AI_GET => {
            let Some(c) = n.academy.inst.offer().and_then(|i| i.courses.get(arg as usize)).cloned() else { return true };
            take(n, ui, c);
        }
        CMD_AI_UPDATE => {
            let Some(e) = n.academy.entries.get(arg as usize) else { return true };
            let Some(c) = n.academy.inst.offer().and_then(|i| i.courses.iter().find(|c| c.id == e.id)).cloned() else { return true };
            take(n, ui, c);
        }
        CMD_AI_REMOVE_ASK => {
            let a = &mut n.academy;
            let Some(e) = a.entries.get(arg as usize) else { return true };
            let title = e.course.as_ref().map_or_else(|| Text::new(&e.id, &e.id), |c| c.title.clone());
            a.inst.remove = Some((e.id.clone(), title));
            a.inst.ask_remove.set(1);
        }
        CMD_AI_REMOVE => {
            let Some(root) = root_of(&n.academy) else { return true };
            let Some(cid) = n.academy.inst.remove.as_ref().map(|x| x.0.clone()) else { return true };
            start(n, ui, What::Remove, None, Work::Remove { root, id: cid });
        }
        CMD_AI_FOLDER => {
            let f = &mut n.file;
            f.fd_folder = true;
            f.fd_save = false;
            f.fd_path = n.academy.inst.start_folder.clone();
            f.fd_active = true;
            f.fd_open.set(1);
        }
        CMD_AI_FOLDER_DONE => {
            let dir = n.file.fd_path.clone();
            let Some(root) = root_of(&n.academy) else { return true };
            start(n, ui, What::Folder, Some(dir.clone()), Work::Read { from: Spec::Folder(dir), root, keep: None });
        }
        CMD_AI_FOLDER_CLOSE => {
            n.academy.inst.folder = None;
            n.academy.focus = ID_AI_FOLDER;
        }
        CMD_AI_CANCEL => {
            if let Some(j) = &n.academy.inst.job {
                j.st.c.stop();
                GATE_STOP.store(true, SeqCst);
            }
        }
        CMD_AI_LINK => {
            let link = n.academy.inst.link.clone();
            if ui.open_url(&link) != 0 {
                n.file.msg = Some((IB_WARNING, String::from(t("Der Browser ließ sich nicht öffnen")), link));
            }
        }
        CMD_AI_GATE => {
            if n.academy.inst.gate {
                GATE_ALLOW.fetch_add(arg, SeqCst);
            }
        }
        _ => return false,
    }
    ui.repass();
    true
}

/// a lesson's web link: nothing opens before the answer to a question that shows the address
pub(crate) fn ask_link(n: &mut Net, ui: &mut Ui, url: &str) {
    n.academy.inst.link = String::from(url);
    n.academy.inst.ask_link.set(1);
    ui.repass();
}

// ---------------------------------------------------------------- the page

/// "412 B", "12 KB", "1,5 MB"
pub(crate) fn size_text(b: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "GB"), (1 << 20, "MB"), (1 << 10, "KB")];
    for (u, name) in UNITS {
        if b >= u {
            let tenths = b.saturating_mul(10) / u;
            if tenths >= 100 || tenths % 10 == 0 {
                return format!("{} {}", tenths / 10, name);
            }
            return format!("{}{}{} {}", tenths / 10, if tr::en() { '.' } else { ',' }, tenths % 10, name);
        }
    }
    format!("{} B", b)
}

// a line of what the worker does, with its bar and Abbrechen
fn job_panel(n: &mut Net, ui: &mut Ui) {
    let Some(j) = n.academy.inst.job.as_ref() else { return };
    let (what, s, stopping) = (j.what, j.st.c.seen(), j.st.c.stopped());
    let title = String::from(j.st.title.lock().get(tr::en()));
    let (course, courses) = (j.st.course.load(SeqCst), j.st.courses.load(SeqCst));
    let (head, line) = match what {
        What::Search => (String::from(t("Die Kursliste wird geladen")), String::from(COURSE_HOST)),
        What::Folder => (String::from(t("Der Ordner wird gelesen")), String::new()),
        What::Remove => (String::from(t("Der Kurs wird deinstalliert")), String::new()),
        What::Install if course == 0 => (String::from(t("Die Kursliste wird geladen")), String::from(COURSE_HOST)),
        What::Install => {
            let head = if courses > 1 { tf("„{}“ wird installiert (Kurs {} von {})", &[&title, &course.to_string(), &courses.to_string()]) } else { tf("„{}“ wird installiert", &[&title]) };
            let line = match s.phase {
                Phase::Fetching => tf("Datei {} von {} · {} von {}", &[&s.file.to_string(), &s.files.to_string(), &size_text(s.done), &size_text(s.total)]),
                Phase::Checking => String::from(t("Der Kurs wird geprüft …")),
                Phase::Placing => String::from(t("Der Kurs kommt an seinen Platz …")),
            };
            (head, line)
        }
    };
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 8);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 2);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, &head);
    if stopping {
        ui.text(TS_CAPTION | TX_SECONDARY, t("Wird abgebrochen …"));
    } else if !line.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &line);
    }
    ui.box_end();
    ui.next_flags(if stopping { ST_DISABLED } else { 0 });
    if ui.button(ID_AI_CANCEL, t("Abbrechen")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AI_CANCEL, 0);
    }
    ui.box_end();
    ui.next_size(-1, 0);
    let per_mille = (s.done.min(s.total).saturating_mul(1000) / s.total.max(1)) as i32;
    ui.progress(ID_AI_PROGRESS, per_mille, 1000);
    ui.box_end();
}

// what the last worker has to say, wrapped, so that what to do is all there
fn note_panel(n: &mut Net, ui: &mut Ui) {
    let Some((sev, title, text)) = n.academy.inst.note.clone() else { return };
    let (icon, tok) = match sev {
        IB_SUCCESS => (IC_CHECK_CIRCLE_FILL, T_OK),
        IB_ERROR => (IC_ERROR_FILL, T_CRIT),
        IB_WARNING => (IC_WARNING, T_WARN),
        _ => (IC_INFO, T_INFO),
    };
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 4);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    mark(ui, icon, tok, 18);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, &title);
    if ui.icon_button(ID_AI_NOTE_CLOSE, IC_CANCEL, t("Schließen")) & EV_ACT != 0 {
        n.academy.inst.note = None;
        ui.repass();
    }
    ui.box_end();
    for line in text.lines() {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, line);
    }
    ui.box_end();
}

/// on the list of courses: the two ways to more of them, what a worker is doing or has to say
pub(crate) fn tools(n: &mut Net, ui: &mut Ui) {
    let busy = n.academy.inst.busy();
    let off = if busy { ST_DISABLED } else { 0 };
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_flags(off);
    ui.next_icon(IC_REFRESH);
    if ui.button(ID_AI_SEARCH, t("Nach Kursen suchen")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AI_SEARCH, 0);
    }
    ui.tooltip(t("Lädt die aktuelle Kursliste; erst dieser Klick verbindet das Programm mit dem Kursserver."));
    ui.next_flags(off);
    ui.next_icon(IC_FOLDER_OPEN);
    if ui.button(ID_AI_FOLDER, t("Aus Ordner installieren …")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AI_FOLDER, 0);
    }
    ui.tooltip(t("Für Autoren und ohne Netz: ein Ordner mit index.json und den Ordnern der Kurse."));
    ui.box_end();
    job_panel(n, ui);
    note_panel(n, ui);
}

/// what the empty list offers
pub(crate) fn empty(n: &mut Net, ui: &mut Ui) {
    ui.next_flags(if n.academy.inst.busy() { ST_DISABLED } else { 0 });
    ui.next_icon(IC_DOWNLOAD);
    if ui.accent_button(ID_AI_ALL, t("Jetzt installieren")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AI_ALL, 0);
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP | TX_CENTER, t("Lädt die Kurse von GitHub. Das Programm verbindet sich erst nach diesem Klick und fragt vorher."));
}

/// a course against the list on the page: how it stands, what the list has of it
pub(crate) fn standing_of<'a>(a: &'a Academy, e: &Entry) -> (Standing, Option<&'a IndexCourse>) {
    let listed = a.inst.offer().and_then(|i| i.courses.iter().find(|c| c.id == e.id));
    (standing(Some(e.course.as_ref().map_or(0, |c| c.course_version)), listed.map(|c| c.version)), listed)
}

/// "Version 2 · 34 KB · aktuell"
pub(crate) fn state_text(a: &Academy, e: &Entry) -> String {
    let (st, listed) = standing_of(a, e);
    let state = match st {
        Standing::Update => tf("Version {} verfügbar", &[&listed.map_or(0, |c| c.version).to_string()]),
        Standing::Newer => String::from(t("neuer als die Kursliste")),
        Standing::Unlisted if a.inst.offer().is_some() => String::from(t("nicht in der Kursliste")),
        Standing::Current => String::from(t("aktuell")),
        _ => String::new(),
    };
    let version = e.course.as_ref().map_or(0, |c| c.course_version).to_string();
    let head = format!("{} · {}", tf("Version {}", &[&version]), size_text(e.bytes));
    if state.is_empty() { head } else { format!("{} · {}", head, state) }
}

/// under a course's card: its version, size and state, Aktualisieren, Deinstallieren
pub(crate) fn row(n: &mut Net, ui: &mut Ui, i: usize) {
    let a = &n.academy;
    let Some(e) = a.entries.get(i) else { return };
    let (st, listed) = standing_of(a, e);
    let line = state_text(a, e);
    let broken = e.usable().is_none();
    // from a folder or over a broken course the same version can come again
    let again = listed.is_some() && (broken || (a.inst.folder.is_some() && matches!(st, Standing::Current | Standing::Newer)));
    let off = if a.inst.busy() { ST_DISABLED } else { 0 };
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.spacer(4);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &line);
    if st == Standing::Update && !broken {
        ui.next_flags(off | ST_COMPACT);
        ui.next_icon(IC_DOWNLOAD);
        if ui.accent_button(ID_AI_UPDATE + i as u32, t("Aktualisieren")) & EV_ACT != 0 {
            ui.cmd_run(CMD_AI_UPDATE, i as u32);
        }
    } else if again {
        ui.next_flags(off | ST_COMPACT);
        ui.next_icon(IC_DOWNLOAD);
        if ui.button(ID_AI_UPDATE + i as u32, t("Erneut installieren")) & EV_ACT != 0 {
            ui.cmd_run(CMD_AI_UPDATE, i as u32);
        }
    }
    ui.next_flags(off | ST_COMPACT | ST_SUBTLE);
    ui.next_icon(IC_DELETE);
    if ui.button(ID_AI_REMOVE + i as u32, t("Deinstallieren")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AI_REMOVE_ASK, i as u32);
    }
    ui.box_end();
}

// a course the list offers and that is not here
fn offer_row(ui: &mut Ui, c: &IndexCourse, k: usize, part: bool, busy: bool) {
    let en = tr::en();
    let meta = format!("{} · {} · {}", level_text(c.level, c.minutes), tf("Version {}", &[&c.version.to_string()]), size_text(c.total()));
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_CARD | pad(12), 12);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 2);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY_STRONG, c.title.get(en));
    if c.sample {
        ui.badge(BADGE_NEUTRAL, t("Beispiel"));
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, c.summary.get(en));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &if part { format!("{} · {}", meta, t("angefangen, es geht dort weiter")) } else { meta });
    ui.box_end();
    ui.next_flags(if busy { ST_DISABLED } else { 0 });
    ui.next_icon(IC_DOWNLOAD);
    if ui.button(ID_AI_GET + k as u32, t("Jetzt installieren")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AI_GET, k as u32);
    }
    ui.tooltip(c.title.get(en));
    ui.box_end();
}

/// what the list on the page offers that is not here; without a list, why there is none
pub(crate) fn offered(n: &mut Net, ui: &mut Ui) {
    let a = &n.academy;
    let root = a.place.courses.clone().unwrap_or_default();
    let busy = a.inst.busy();
    let folder = a.inst.folder.as_ref().map(|x| x.0.clone());
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 8);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    ui.text(TS_SUBTITLE | TX_ELLIPSIS, t("Verfügbar"));
    if folder.is_some() {
        ui.next_flags(ST_SUBTLE | ST_COMPACT);
        ui.next_icon(IC_CANCEL);
        if ui.button(ID_AI_FOLDER_CLOSE, t("Ordner schließen")) & EV_ACT != 0 {
            ui.cmd_run(CMD_AI_FOLDER_CLOSE, 0);
        }
    }
    ui.box_end();
    if let Some(dir) = &folder {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("Aus dem Ordner {}", &[dir]));
    }
    match a.inst.offer() {
        None => {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Noch keine Kursliste geladen. Erst „Nach Kursen suchen“ verbindet das Programm mit dem Kursserver; installierte Kurse brauchen kein Netz."));
        }
        Some(i) => {
            let rows: Vec<(usize, &IndexCourse)> = i.courses.iter().enumerate().filter(|(_, c)| !a.entries.iter().any(|e| e.id == c.id)).collect();
            if rows.is_empty() {
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Alle Kurse dieser Kursliste sind installiert."));
            }
            for (k, c) in rows {
                offer_row(ui, c, k, way::has_part(&root, &c.id), busy);
            }
        }
    }
    ui.box_end();
}

/// "Kurs deinstallieren?", "Diese Adresse öffnen?"
pub(crate) fn dialog(n: &mut Net, ui: &mut Ui) {
    let open = n.academy.inst.ask_remove.clone();
    if ui.dialog_begin(ID_AI_REMOVE_DLG, t("Kurs deinstallieren?"), &open) {
        let a = &n.academy;
        let (id, title) = a.inst.remove.as_ref().map_or((String::new(), String::new()), |x| (x.0.clone(), String::from(x.1.get(tr::en()))));
        let lab_open = a.lab.as_ref().and_then(|l| a.entries.get(l.course)).is_some_and(|e| e.id == id);
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP, &tf("„{}“ wird von diesem Rechner entfernt. Ihr Fortschritt bleibt erhalten und gilt wieder, wenn Sie den Kurs erneut installieren.", &[&title]));
        if lab_open {
            ui.next_size(420, 0);
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Das offene Labor dieses Kurses bleibt als Projekt geöffnet, nur ohne die Leiste des Kurses."));
        }
        if ui.dialog_buttons(Some(t("Deinstallieren")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            ui.cmd_run(CMD_AI_REMOVE, 0);
        }
    }
    ui.dialog_end();
    let open = n.academy.inst.ask_link.clone();
    if ui.dialog_begin(ID_AI_LINK_DLG, t("Diese Adresse öffnen?"), &open) {
        ui.next_size(460, 0);
        ui.text(TS_BODY | TX_WRAP, t("Dieser Link führt aus dem Programm heraus in Ihren Browser:"));
        ui.next_size(460, 0);
        ui.text(TS_BODY_STRONG | TX_WRAP, &n.academy.inst.link);
        if ui.dialog_buttons(Some(t("Öffnen")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            ui.cmd_run(CMD_AI_LINK, 0);
        }
    }
    ui.dialog_end();
}

/// what a first install takes (selftest)
pub(crate) fn first_install_of(index: &Index, here: &[(String, u32)]) -> Vec<String> {
    first_install(index, here).into_iter().map(|c| c.id).collect()
}
