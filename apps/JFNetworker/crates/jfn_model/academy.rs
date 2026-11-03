// academy -- the courses of the Akademie as files: what a course is, what is wrong with one, what a
// learner has done.  Text in, records and messages out, no I/O and no clock, so that the app, jfn-sim
// and the selftest read the same files the same way.
//
//   academy/index.json           Index: what can be installed (the installer in crates/jfn_run reads it,
//                                the list of courses comes from the folders)
//   academy/<kurs>/course.json   Course: title, summary, level, time, units with steps
//   academy/<kurs>/de/*.md, en/*.md   the lessons, one folder per language code
//   academy/<kurs>/labs/*.jfnet, img/*   the labs and the pictures
//   academy/progress.json        Progress: per course and step the state, when, the best result
//
//   Index::from_text(text) / Course::from_text(text) -> Result<_, Msg>   the wrong format or a newer
//                                                version is refused with its message; missing members
//                                                take their defaults, unknown ones are skipped
//   load(id, text, &exists) -> (Option<Course>, Vec<Problem>)   a course's folder as a whole: its
//                                                name, its course.json (None: it cannot be read), its files
//   index.issues() / course.issues() -> Vec<Problem>   what is wrong in the file itself; for the index also
//                                its limits: INDEX_MAX bytes, INDEX_COURSES_MAX courses, per course
//                                COURSE_FILES_MAX files of FILE_MAX bytes, COURSE_BYTES_MAX bytes in all
//   standing(installed, offered) -> Standing     a course against the index, by the versions of its content
//   entry.total() -> bytes                       an index entry's files together
//   course.file_issues(&exists) -> Vec<Problem>  the lessons in every language, the labs (exists: a
//                                                path relative to the course's folder)
//   course.lesson_issues(step, text, &exists)    a lesson's pictures and links
//   lab_issues(step, text) -> Vec<Problem>       a lab step's file: a project with a lab and goals
//   fault_issues(step, text) -> Vec<Problem>     a fault hunt's file: a project with a lab it grades (AK2)
//   quiz_issues(step, kind, text) -> Vec<Problem>   a quiz's or test's file (quiz.rs, AK2)
//   Problem { fatal, msg }                       fatal: the course cannot be offered; else a note (a
//                                                lesson in one language only)
//   safe_path(p) -> Result<(), Msg>              relative, without "..", inside the course's folder
//   safe_id(s)                                   a course's or step's id: a-z 0-9 - _
//   image_file(base, addr) -> Option<&str>       the file a picture's address names, if it may be read
//   lesson_refs(md) -> Vec<MdRef> / link_to(target) -> LinkTo   a lesson's pictures and links, and
//                                                where a link leads
//   lesson_files(file, lang) -> [(path, lang); 2]   the lesson's files, the language asked for first
//   step.kind() -> Option<StepKind> / step.ready()   every kind can be opened (AK2); a quiz, a fault hunt
//                                                or an exam without a file is announced, not written yet
//   course.steps() / step_at(i) / index_of(id) / next_ready(i) / prev_ready(i) / ready_count()
//                                                the steps in order, across the units
//   Progress::from_text(text) -> Progress        corrupt or newer: empty, never an error
//   progress.to_text() / is_done(course, step) / mark_done(course, step, when) / clear(course)
//   progress.record(course, step, percent, when) / note_best(course, step, percent) / best_of(course, step)
//   / tries_of(course, step)                     a test's result: the best per cent, how many were handed in
//   progress.states(&course, id) -> Vec<StepState>   Done, Current (the first open step), Open, NotYet,
//                                                Locked (a step it `requires` is open)
//   progress.done_count(&course, id) / next_open(&course, id)
//
// A step's `requires` names earlier steps that must be done before it opens (default: none).
// A step's `file` is relative to the course's folder ("labs/x.jfnet"), a lesson's to the folder of
// its language ("01-netz.md" -> de/01-netz.md, en/01-netz.md): a new language is a new folder.  A
// lesson's pictures are relative to the course's folder ("img/x.svg").  Its links are step:ID, lab:ID
// or an address of the web (which the app opens only after a question that shows the address).
//
// `version` of index.json and course.json is the version of the format; the version of a course's
// content is `version` of its entry in index.json and `course_version` of its course.json.  Progress
// goes by the ids of courses and steps, so it survives a new version of a course as long as they
// stay; the entries of steps that are gone are kept, should they come back.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::{Json, Value};
use agent_core::json;
use agent_core::{record, wire_enum};

use crate::lab::Text;
use crate::msg;
use crate::msg::Msg;
use crate::project::{file_limits, Project};
use crate::quiz::Quiz;

/// `format` of index.json, of course.json and of progress.json
pub const INDEX_FORMAT: &str = "jfn-academy";
pub const COURSE_FORMAT: &str = "jfn-course";
pub const PROGRESS_FORMAT: &str = "jfn-progress";
/// the newest version of these formats this program reads and writes
pub const VERSION: u32 = 1;
/// a course's file in its folder, the learner's file beside the courses
pub const COURSE_FILE: &str = "course.json";
pub const PROGRESS_FILE: &str = "progress.json";
/// the folders of the lessons this program looks for, by language code (the source language first)
pub const LANGS: [&str; 2] = ["de", "en"];
/// the longest path in a course (characters)
const PATH_MAX: usize = 200;
/// the longest id
const ID_MAX: usize = 64;
/// the most steps of a course (the app's block of ids has room for them)
const STEPS_MAX: usize = 1000;
/// what an index may ask for: its own size, its courses, per course the files, a file's bytes and all its bytes
pub const INDEX_MAX: usize = 4 << 20;
pub const INDEX_COURSES_MAX: usize = 512;
pub const COURSE_FILES_MAX: usize = 4096;
pub const FILE_MAX: u64 = 64 << 20;
pub const COURSE_BYTES_MAX: u64 = 512 << 20;

wire_enum! {
    /// who a course is for
    pub enum Level {
        Beginner = "beginner",
        Intermediate = "intermediate",
        Advanced = "advanced",
    }
}

wire_enum! {
    /// what a step is: a lesson to read, a lab with goals, a quiz to practise, a network to repair, a
    /// written test
    pub enum StepKind {
        Lesson = "lesson",
        Lab = "lab",
        Quiz = "quiz",
        Fault = "fault",
        Exam = "exam",
    }
}

impl StepKind {
    /// this program can open it (a newer course may name a kind it does not know: that is no kind at all)
    pub fn ready(self) -> bool {
        true
    }
    /// it needs a file before it can be offered (the others may be announced with no file yet)
    pub fn needs_file(self) -> bool {
        matches!(self, StepKind::Lesson | StepKind::Lab)
    }
}

record! {
    /// a file of a course as index.json lists it (AK3 checks and fetches it)
    pub struct FileEntry {
        /// relative to the course's folder
        pub path: String,
        pub size: u64,
        /// lower-case hex
        pub sha256: String,
    }
}

record! {
    /// a course as index.json lists it; its folder is its id
    pub struct IndexCourse {
        pub id: String,
        pub title: Text,
        pub summary: Text,
        pub level: Level,
        pub minutes: u32,
        /// the version of its content (course_version of its course.json)
        pub version: u32,
        /// a sample that shows the format: offered, but not among the courses a first install takes
        pub sample: bool,
        pub files: Vec<FileEntry>,
    }
}

record! {
    /// academy/index.json
    pub struct Index {
        pub format: String,
        pub version: u32,
        pub courses: Vec<IndexCourse>,
    }
}

record! {
    /// a step of a course
    pub struct Step {
        /// what progress and the links go by; stays when the course is updated
        pub id: String,
        /// "lesson", ...: StepKind::parse; an unknown one is a fatal problem
        pub kind: String,
        pub title: Text,
        pub file: String,
        /// earlier steps that must be done before this one opens
        pub requires: Vec<String>,
    }
}

record! {
    /// a part of a course
    pub struct Unit {
        pub title: Text,
        pub steps: Vec<Step>,
    }
}

record! {
    /// academy/<kurs>/course.json
    pub struct Course {
        pub format: String,
        pub version: u32,
        /// the version of the content
        pub course_version: u32,
        /// a sample that shows the format (the index says so too)
        pub sample: bool,
        pub title: Text,
        pub summary: Text,
        pub level: Level,
        pub minutes: u32,
        pub units: Vec<Unit>,
    }
}

/// something wrong with a course
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// the course cannot be offered (else it can, and this is a note for its author)
    pub fatal: bool,
    pub msg: Msg,
}

pub(crate) fn fatal(msg: Msg) -> Problem {
    Problem { fatal: true, msg }
}

pub(crate) fn note(msg: Msg) -> Problem {
    Problem { fatal: false, msg }
}

// the JSON of a file (a byte order mark is no error)
pub(crate) fn parse(text: &str) -> Result<Value, Msg> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    json::parse_with_limits(text.as_bytes(), file_limits()).map_err(|e| msg!("Die Datei ist kein gültiges JSON ({} an Byte {}).", e.kind, e.offset))
}

pub(crate) fn version_ok(v: u32) -> Result<(), Msg> {
    if v == 0 || v > VERSION {
        return Err(msg!("Die Datei hat die Version {}; dieses Programm liest Version 1 bis {}.", v, VERSION));
    }
    Ok(())
}

// a name Windows keeps for a device, in any folder and with any ending
fn reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_lowercase();
    let b = stem.as_bytes();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul") || (b.len() == 4 && (b.starts_with(b"com") || b.starts_with(b"lpt")) && b[3].is_ascii_digit())
}

/// a path of a course: relative, without "..", no drive and no scheme, letters, digits and - _ . only,
/// so that it stays inside the course's folder on every system
pub fn safe_path(p: &str) -> Result<(), Msg> {
    if p.is_empty() {
        return Err(msg!("Der Pfad ist leer."));
    }
    if p.starts_with(['/', '\\']) || p.contains(':') {
        return Err(msg!("Der Pfad „{}“ ist absolut; erlaubt sind nur Pfade im Ordner des Kurses.", p));
    }
    if p.split(['/', '\\']).any(|s| s == "..") {
        return Err(msg!("Der Pfad „{}“ führt mit „..“ aus dem Ordner des Kurses.", p));
    }
    if p.chars().count() > PATH_MAX || !p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/')) {
        return Err(msg!("Der Pfad „{}“ ist zu lang oder enthält andere Zeichen als Buchstaben, Ziffern und - _ . /", p));
    }
    if p.split('/').any(|s| s.is_empty() || s.starts_with('.') || s.ends_with('.') || reserved(s)) {
        return Err(msg!("Der Pfad „{}“ hat einen leeren oder unzulässigen Teil.", p));
    }
    Ok(())
}

/// an id of a course or step: small letters, digits, - and _, a letter or digit first
pub fn safe_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= ID_MAX && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_')) && !reserved(s)
}

/// the file inside the course's folder (`base`, with its last '/') that a picture's address names:
/// None for one outside it, of another kind, or of no file at all
pub fn image_file<'a>(base: &str, addr: &'a str) -> Option<&'a str> {
    let rel = addr.strip_prefix(base)?;
    safe_path(rel).ok()?;
    let low = rel.to_ascii_lowercase();
    ["png", "jpg", "jpeg", "gif", "bmp", "svg"].iter().any(|e| low.rsplit('.').next() == Some(*e)).then_some(rel)
}

/// the files of a lesson (its `file` in the folders of the languages) with their language code, the
/// one asked for first
pub fn lesson_files(file: &str, lang: &str) -> [(String, &'static str); 2] {
    let first = LANGS.iter().position(|l| *l == lang).unwrap_or(0);
    let at = |k: usize| {
        let l = LANGS[k];
        (format!("{}/{}", l, file), l)
    };
    [at(first), at(1 - first)]
}

/// a picture or a link in a lesson
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdRef {
    pub image: bool,
    pub target: String,
}

/// the pictures and links of a Markdown text, outside code (the app's reader sees more, the links of
/// raw HTML for one; this is what an author writes)
pub fn lesson_refs(md: &str) -> Vec<MdRef> {
    let mut v = Vec::new();
    let mut fence = false;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        let b = line.as_bytes();
        // the open brackets of this line: where, and whether a '!' stands before it
        let mut open: Vec<bool> = Vec::new();
        let (mut i, mut code) = (0, false);
        while i < b.len() {
            match b[i] {
                b'\\' => i += 1,
                b'`' => code = !code,
                b'[' if !code => open.push(i > 0 && b[i - 1] == b'!'),
                b']' if !code => {
                    let image = open.pop();
                    if let (Some(image), Some(b'(')) = (image, b.get(i + 1)) {
                        let rest = &line[i + 2..];
                        let Some(end) = rest.find(')') else { break };
                        let inner = rest[..end].trim();
                        // a title may follow the address; <...> may enclose it
                        let target = inner.split_whitespace().next().unwrap_or("").trim_start_matches('<').trim_end_matches('>');
                        v.push(MdRef { image, target: String::from(target) });
                        i += 1 + end;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    v
}

/// where a lesson's link leads
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkTo<'a> {
    /// step:ID, the step of that id
    Step(&'a str),
    /// lab:ID, the lab step of that id
    Lab(&'a str),
    /// http: or https:
    Web,
    /// nowhere the app goes
    Other,
}

/// where a link's target leads (the prefixes as written)
pub fn link_to(target: &str) -> LinkTo<'_> {
    if let Some(id) = target.strip_prefix("step:") {
        return LinkTo::Step(id);
    }
    if let Some(id) = target.strip_prefix("lab:") {
        return LinkTo::Lab(id);
    }
    let low = target.get(..8).unwrap_or(target).to_ascii_lowercase();
    if low.starts_with("http://") || low.starts_with("https://") { LinkTo::Web } else { LinkTo::Other }
}

/// a lab step's file: a project that loads, with a lab and goals the simulator can check
pub fn lab_issues(step: &str, text: &str) -> Vec<Problem> {
    let p = match Project::from_text(text) {
        Ok(p) => p,
        Err(e) => return alloc::vec![fatal(msg!("Schritt {}: {}", step, e.msg))],
    };
    let Some(lab) = &p.lab else {
        return alloc::vec![fatal(msg!("Schritt {}: die Datei hat keine Aufgabe.", step))];
    };
    let mut v: Vec<Problem> = lab.issues(&p).into_iter().map(|m| fatal(msg!("Schritt {}: {}", step, m))).collect();
    if lab.goals.is_empty() {
        v.push(fatal(msg!("Schritt {}: das Labor hat keine Ziele, an denen sich sein Ende erkennen lässt.", step)));
    }
    v
}

/// a fault hunt's file: a project that loads, with a lab that grades (handing in decides, not goals)
pub fn fault_issues(step: &str, text: &str) -> Vec<Problem> {
    let p = match Project::from_text(text) {
        Ok(p) => p,
        Err(e) => return alloc::vec![fatal(msg!("Schritt {}: {}", step, e.msg))],
    };
    let Some(lab) = &p.lab else {
        return alloc::vec![fatal(msg!("Schritt {}: die Datei hat keine Aufgabe.", step))];
    };
    let mut v: Vec<Problem> = lab.issues(&p).into_iter().map(|m| fatal(msg!("Schritt {}: {}", step, m))).collect();
    if lab.assess.is_none() {
        v.push(fatal(msg!("Schritt {}: eine Fehlersuche wird bewertet; die Datei hat keine Kriterien (assess).", step)));
    }
    v
}

/// a quiz's or test's file; a time limit or a limit of attempts means nothing in practice
pub fn quiz_issues(step: &str, kind: StepKind, text: &str) -> Vec<Problem> {
    let q = match Quiz::from_text(text) {
        Ok(q) => q,
        Err(m) => return alloc::vec![fatal(msg!("Schritt {}: {}", step, m))],
    };
    let mut v: Vec<Problem> = q.issues().into_iter().map(|p| Problem { fatal: p.fatal, msg: msg!("Schritt {}: {}", step, p.msg) }).collect();
    if kind == StepKind::Quiz && (q.minutes > 0 || q.attempts > 0) {
        v.push(note(msg!("Schritt {}: Zeitlimit und Versuche gelten nur in einem Test (kind „exam“).", step)));
    }
    v
}

impl Step {
    /// its kind; None: one this program does not know
    pub fn kind(&self) -> Option<StepKind> {
        StepKind::parse(&self.kind)
    }
    /// this program can open it: a known kind with its file (a quiz without a file is only announced)
    pub fn ready(&self) -> bool {
        self.kind().is_some_and(|k| k.ready() && (k.needs_file() || !self.file.is_empty()))
    }
}

impl Course {
    /// the text of course.json -> the course, or why not
    pub fn from_text(text: &str) -> Result<Course, Msg> {
        let c = Course::from_json(&parse(text)?).ok_or_else(|| msg!("Die Datei enthält keinen Kurs."))?;
        if c.format != COURSE_FORMAT {
            return Err(msg!("Das ist keine Kursdatei."));
        }
        version_ok(c.version)?;
        Ok(c)
    }

    /// the steps in order, across the units
    pub fn steps(&self) -> impl Iterator<Item = &Step> {
        self.units.iter().flat_map(|u| u.steps.iter())
    }

    pub fn step_at(&self, i: usize) -> Option<&Step> {
        self.steps().nth(i)
    }

    /// the place of a step in the order
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.steps().position(|s| s.id == id)
    }

    /// the next step after i this program can open
    pub fn next_ready(&self, i: usize) -> Option<usize> {
        self.steps().enumerate().skip(i + 1).find(|(_, s)| s.ready()).map(|(k, _)| k)
    }

    /// the last step before i this program can open
    pub fn prev_ready(&self, i: usize) -> Option<usize> {
        self.steps().enumerate().take(i).filter(|(_, s)| s.ready()).last().map(|(k, _)| k)
    }

    /// how many steps this program can open
    pub fn ready_count(&self) -> usize {
        self.steps().filter(|s| s.ready()).count()
    }

    /// what is wrong in the file itself: titles, ids, kinds, paths
    pub fn issues(&self) -> Vec<Problem> {
        let mut v = Vec::new();
        if self.title.is_empty() {
            v.push(fatal(msg!("Der Kurs hat keinen Titel.")));
        }
        if self.steps().next().is_none() {
            v.push(fatal(msg!("Der Kurs hat keine Schritte.")));
        }
        if self.steps().count() > STEPS_MAX {
            v.push(fatal(msg!("Der Kurs hat mehr als {} Schritte.", STEPS_MAX)));
        }
        if self.course_version == 0 {
            v.push(note(msg!("Der Kurs hat keine Versionsnummer (course_version).")));
        }
        for t in [&self.title, &self.summary].into_iter().chain(self.units.iter().map(|u| &u.title)) {
            if !t.is_empty() && t.en.is_empty() {
                v.push(note(msg!("„{}“ hat keinen englischen Text.", t.de)));
            }
        }
        for (k, u) in self.units.iter().enumerate() {
            if u.title.is_empty() {
                v.push(note(msg!("Einheit {} hat keinen Titel.", k + 1)));
            }
        }
        let mut seen: Vec<&str> = Vec::new();
        for (k, s) in self.steps().enumerate() {
            v.extend(step_issues(s, k + 1));
            if seen.contains(&s.id.as_str()) {
                v.push(fatal(msg!("Die Kennung {} kommt doppelt vor.", s.id)));
            }
            for r in s.requires.iter().filter(|r| !seen.contains(&r.as_str())) {
                v.push(fatal(msg!("Schritt {}: der verlangte Schritt {} steht nicht davor.", s.id, r)));
            }
            seen.push(&s.id);
        }
        v
    }

    /// the files of the steps: every lesson in every language (one language missing: a note, the
    /// other shows), every lab, quiz, test and fault hunt with a file; `exists` takes a path relative to
    /// the course's folder
    pub fn file_issues(&self, exists: &dyn Fn(&str) -> bool) -> Vec<Problem> {
        let mut v = Vec::new();
        for s in self.steps().filter(|s| safe_path(&s.file).is_ok()) {
            match s.kind() {
                Some(StepKind::Lesson) => {
                    let missing: Vec<&str> = LANGS.iter().copied().filter(|l| !exists(&format!("{}/{}", l, s.file))).collect();
                    if missing.len() == LANGS.len() {
                        v.push(fatal(msg!("Schritt {}: die Lektion {} fehlt in jeder Sprache.", s.id, s.file)));
                    } else {
                        for l in missing {
                            v.push(note(msg!("Schritt {}: die Lektion {} fehlt im Ordner {}; es erscheint die andere Sprache.", s.id, s.file, l)));
                        }
                    }
                }
                Some(StepKind::Lab) if !exists(&s.file) => v.push(fatal(msg!("Schritt {}: die Datei {} fehlt.", s.id, s.file))),
                Some(StepKind::Quiz | StepKind::Exam | StepKind::Fault) if !s.file.is_empty() && !exists(&s.file) => v.push(fatal(msg!("Schritt {}: die Datei {} fehlt.", s.id, s.file))),
                _ => {}
            }
        }
        v
    }

    /// a lesson's text: its pictures (in the course's folder, there), its links (to a step that exists,
    /// a lab: to a lab)
    pub fn lesson_issues(&self, step: &str, text: &str, exists: &dyn Fn(&str) -> bool) -> Vec<Problem> {
        let mut v = Vec::new();
        for r in lesson_refs(text) {
            if r.image {
                match safe_path(&r.target) {
                    Err(m) => v.push(note(msg!("Schritt {}: {}", step, m))),
                    Ok(()) if !exists(&r.target) => v.push(note(msg!("Schritt {}: das Bild {} fehlt.", step, r.target))),
                    Ok(()) => {}
                }
                continue;
            }
            match link_to(&r.target) {
                LinkTo::Step(id) if self.index_of(id).is_none() => v.push(note(msg!("Schritt {}: den Schritt {} gibt es nicht.", step, id))),
                LinkTo::Lab(id) if self.steps().find(|s| s.id == id).and_then(Step::kind) != Some(StepKind::Lab) => v.push(note(msg!("Schritt {}: {} ist kein Labor.", step, id))),
                LinkTo::Other => v.push(note(msg!("Schritt {}: der Link „{}“ führt nirgendwohin; es gibt step:, lab:, http: und https:.", step, r.target))),
                _ => {}
            }
        }
        v
    }
}

/// a course's folder as a whole: `id` its name, `text` its course.json (None: it cannot be read), `exists`
/// takes a path relative to the folder -> the course if it parses, and everything wrong with it
pub fn load(id: &str, text: Option<&str>, exists: &dyn Fn(&str) -> bool) -> (Option<Course>, Vec<Problem>) {
    let mut v = Vec::new();
    if !safe_id(id) {
        v.push(fatal(msg!("Die Kennung „{}“ darf nur aus kleinen Buchstaben, Ziffern, - und _ bestehen.", id)));
    }
    let Some(text) = text else {
        v.push(fatal(msg!("Die Datei {} fehlt oder lässt sich nicht lesen.", COURSE_FILE)));
        return (None, v);
    };
    match Course::from_text(text) {
        Ok(c) => {
            v.extend(c.issues());
            v.extend(c.file_issues(exists));
            (Some(c), v)
        }
        Err(m) => {
            v.push(fatal(m));
            (None, v)
        }
    }
}

// what is wrong with one step (k: its number in the course)
fn step_issues(s: &Step, k: usize) -> Vec<Problem> {
    let mut v = Vec::new();
    if !safe_id(&s.id) {
        v.push(fatal(msg!("Schritt {}: die Kennung „{}“ darf nur aus kleinen Buchstaben, Ziffern, - und _ bestehen.", k, s.id)));
    }
    if s.title.is_empty() {
        v.push(fatal(msg!("Schritt {} hat keinen Titel.", k)));
    } else if s.title.en.is_empty() {
        v.push(note(msg!("„{}“ hat keinen englischen Text.", s.title.de)));
    }
    let Some(kind) = s.kind() else {
        v.push(fatal(msg!("Schritt {}: die Art „{}“ gibt es nicht (lesson, lab, quiz, fault, exam).", k, s.kind)));
        return v;
    };
    // a quiz, a test or a fault hunt may wait for its file, but no path of a course leaves its folder
    if kind.needs_file() || !s.file.is_empty() {
        if let Err(m) = safe_path(&s.file) {
            v.push(fatal(msg!("Schritt {}: {}", k, m)));
            return v;
        }
    }
    let wanted = match kind {
        StepKind::Lesson => Some(".md"),
        StepKind::Lab | StepKind::Fault => Some(".jfnet"),
        StepKind::Quiz | StepKind::Exam => Some(".json"),
    };
    if let Some(end) = wanted.filter(|e| !s.file.is_empty() && !s.file.ends_with(e)) {
        v.push(fatal(msg!("Schritt {}: die Datei {} endet nicht auf {}.", k, s.file, end)));
    }
    v
}

impl Index {
    /// the text of index.json -> the list, or why not
    pub fn from_text(text: &str) -> Result<Index, Msg> {
        let i = Index::from_json(&parse(text)?).ok_or_else(|| msg!("Die Datei enthält keine Kursliste."))?;
        if i.format != INDEX_FORMAT {
            return Err(msg!("Das ist keine Kursliste."));
        }
        version_ok(i.version)?;
        Ok(i)
    }

    /// what is wrong in the list: ids, paths, checksums, its limits
    pub fn issues(&self) -> Vec<Problem> {
        let mut v = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        if self.courses.len() > INDEX_COURSES_MAX {
            v.push(fatal(msg!("Die Kursliste nennt mehr als {} Kurse.", INDEX_COURSES_MAX)));
        }
        for c in &self.courses {
            if !safe_id(&c.id) {
                v.push(fatal(msg!("Die Kennung „{}“ darf nur aus kleinen Buchstaben, Ziffern, - und _ bestehen.", c.id)));
            }
            if seen.contains(&c.id.as_str()) {
                v.push(fatal(msg!("Die Kennung {} kommt doppelt vor.", c.id)));
            }
            seen.push(&c.id);
            if !c.files.iter().any(|f| f.path == COURSE_FILE) {
                v.push(fatal(msg!("Kurs {}: die Kursliste nennt keine Datei {}.", c.id, COURSE_FILE)));
            }
            if c.files.len() > COURSE_FILES_MAX {
                v.push(fatal(msg!("Kurs {}: die Kursliste nennt mehr als {} Dateien.", c.id, COURSE_FILES_MAX)));
            }
            if c.total() > COURSE_BYTES_MAX {
                v.push(fatal(msg!("Kurs {}: er ist mit {} Bytes größer als die erlaubten {} Bytes.", c.id, c.total(), COURSE_BYTES_MAX)));
            }
            let mut names: Vec<String> = Vec::new();
            for f in &c.files {
                if let Err(m) = safe_path(&f.path) {
                    v.push(fatal(msg!("Kurs {}: {}", c.id, m)));
                }
                // the same name to a file system that ignores case
                let low = f.path.to_ascii_lowercase();
                if names.contains(&low) {
                    v.push(fatal(msg!("Kurs {}: die Datei {} kommt doppelt vor.", c.id, f.path)));
                }
                names.push(low);
                if f.size > FILE_MAX {
                    v.push(fatal(msg!("Kurs {}: die Datei {} ist mit {} Bytes größer als die erlaubten {} Bytes.", c.id, f.path, f.size, FILE_MAX)));
                }
                if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                    v.push(fatal(msg!("Kurs {}: die Prüfsumme von {} ist kein SHA-256.", c.id, f.path)));
                }
            }
        }
        v
    }
}

impl IndexCourse {
    /// the bytes of its files together
    pub fn total(&self) -> u64 {
        self.files.iter().fold(0u64, |a, f| a.saturating_add(f.size))
    }
}

/// how a course stands against the index
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// the index offers it, it is not here
    NotInstalled,
    /// here in the version the index offers
    Current,
    /// the index has a newer version
    Update,
    /// here in a newer version than the index offers (an author's folder, say): no way down
    Newer,
    /// here, and the index does not name it (or there is no index)
    Unlisted,
}

/// a course against the index: the version of its content here (`course_version`) and in the index
/// (`version`); None: not here, not in the index
pub fn standing(installed: Option<u32>, offered: Option<u32>) -> Standing {
    match (installed, offered) {
        (None, _) => Standing::NotInstalled,
        (Some(_), None) => Standing::Unlisted,
        (Some(a), Some(b)) if b > a => Standing::Update,
        (Some(a), Some(b)) if a > b => Standing::Newer,
        _ => Standing::Current,
    }
}

// ---------------------------------------------------------------- what a learner has done

wire_enum! {
    /// what the file says about a step
    pub enum StepMark {
        Open = "open",
        Done = "done",
    }
}

record! {
    /// a step of a learner
    pub struct StepRecord {
        pub state: StepMark,
        /// when it was done (seconds since 1970)
        pub when: i64,
        /// the best result of a test in per cent (0: none, or `tries` say it was 0 %)
        pub best: u32,
        /// how many attempts of a test were handed in
        pub tries: u32,
    }
}

record! {
    /// a course of a learner
    pub struct CourseRecord {
        pub steps: BTreeMap<String, StepRecord>,
    }
}

record! {
    /// academy/progress.json
    pub struct Progress {
        pub format: String,
        pub version: u32,
        pub courses: BTreeMap<String, CourseRecord>,
    }
}

/// how a step stands for a learner
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    /// the first step that is open: where "Weiter" goes
    Current,
    Open,
    /// announced, its file is not written yet
    NotYet,
    /// a step it requires is open
    Locked,
}

impl Progress {
    /// the text of progress.json; corrupt, of another format or newer: empty
    pub fn from_text(text: &str) -> Progress {
        let read = parse(text).ok().and_then(|v| Progress::from_json(&v));
        read.filter(|p| p.format == PROGRESS_FORMAT && version_ok(p.version).is_ok()).unwrap_or_default()
    }

    /// the text of progress.json
    pub fn to_text(&self) -> String {
        let mut p = self.clone();
        p.format = String::from(PROGRESS_FORMAT);
        p.version = VERSION;
        let mut s = json::stringify_pretty(&p.to_json(), file_limits(), 2).unwrap_or_default();
        s.push('\n');
        s
    }

    pub fn is_done(&self, course: &str, step: &str) -> bool {
        self.courses.get(course).and_then(|c| c.steps.get(step)).is_some_and(|s| s.state == StepMark::Done)
    }

    /// the step is done (`when`: seconds since 1970) -> this changed something
    pub fn mark_done(&mut self, course: &str, step: &str, when: i64) -> bool {
        if self.is_done(course, step) {
            return false;
        }
        let r = self.courses.entry(String::from(course)).or_default().steps.entry(String::from(step)).or_default();
        r.state = StepMark::Done;
        r.when = when;
        true
    }

    /// an attempt handed in: one more try, the best result (`when`: seconds since 1970, if it is the
    /// first thing known of the step)
    pub fn record(&mut self, course: &str, step: &str, percent: u32, when: i64) {
        let r = self.courses.entry(String::from(course)).or_default().steps.entry(String::from(step)).or_default();
        r.tries = r.tries.saturating_add(1);
        r.best = r.best.max(percent);
        if r.when == 0 {
            r.when = when;
        }
    }

    /// what practice reached so far -> this changed something
    pub fn note_best(&mut self, course: &str, step: &str, percent: u32) -> bool {
        if percent == 0 || self.best_of(course, step).is_some_and(|b| b >= percent) {
            return false;
        }
        let r = self.courses.entry(String::from(course)).or_default().steps.entry(String::from(step)).or_default();
        r.best = percent;
        true
    }

    /// the best result (None: no result yet)
    pub fn best_of(&self, course: &str, step: &str) -> Option<u32> {
        let r = self.courses.get(course)?.steps.get(step)?;
        (r.best > 0 || r.tries > 0).then_some(r.best)
    }

    /// how many attempts were handed in
    pub fn tries_of(&self, course: &str, step: &str) -> u32 {
        self.courses.get(course).and_then(|c| c.steps.get(step)).map_or(0, |r| r.tries)
    }

    /// a step it requires is not done
    pub fn locked(&self, c: &Course, id: &str, s: &Step) -> bool {
        s.requires.iter().any(|r| !self.is_done(id, r) && c.index_of(r).is_some())
    }

    /// nothing of the course counts any more -> this changed something
    pub fn clear(&mut self, course: &str) -> bool {
        self.courses.remove(course).is_some()
    }

    /// how the steps of a course stand, in order
    pub fn states(&self, c: &Course, id: &str) -> Vec<StepState> {
        let mut current = false;
        c.steps()
            .map(|s| {
                if self.is_done(id, &s.id) {
                    StepState::Done
                } else if !s.ready() {
                    StepState::NotYet
                } else if self.locked(c, id, s) {
                    StepState::Locked
                } else if current {
                    StepState::Open
                } else {
                    current = true;
                    StepState::Current
                }
            })
            .collect()
    }

    /// how many of the steps this program can open are done
    pub fn done_count(&self, c: &Course, id: &str) -> usize {
        c.steps().filter(|s| s.ready() && self.is_done(id, &s.id)).count()
    }

    /// the first step that is open
    pub fn next_open(&self, c: &Course, id: &str) -> Option<usize> {
        c.steps().position(|s| s.ready() && !self.is_done(id, &s.id) && !self.locked(c, id, s))
    }
}
