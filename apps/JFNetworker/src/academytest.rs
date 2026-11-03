// academytest -- checks of the Akademie without a window (part of --selftest, group "academy"): the
// files (a good course and index, a newer version, the wrong format, a bad path, a lesson missing in one
// language, an unknown step kind, what a lesson refers to), the progress (a round trip, a corrupt file,
// another course version), step order and state, where a run looks for its files, what the page reads and
// what it refuses; and every course below --academy-dir (tests/model.uis names apps/JFNetworker/academy):
// its files, both languages, its labs with jfn-sim check's findings, unsolved and solved.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::academy::*;
use jfn_model::project::Project;
use jfn_sim::goal::LabRun;
use jfn_sim::{Cmd, SEC};
use ui::MdViewState;

use crate::doc::sim::Sim;
use crate::selftest::T;
use crate::workspace::academy::store::{self, Place};
use crate::workspace::academy::{Academy, Card};

const COURSE: &str = r#"{
  "format": "jfn-course", "version": 1, "course_version": 3, "morgen": {"unbekannt": true},
  "title": ["Kurs", "Course"], "summary": ["Kurz", "Short"], "level": "intermediate", "minutes": 20,
  "units": [
    {"title": ["Eins", "One"], "steps": [
      {"id": "a", "kind": "lesson", "title": ["A", "A"], "file": "a.md"},
      {"id": "b", "kind": "lab", "title": ["B", "B"], "file": "labs/b.jfnet"}]},
    {"title": ["Zwei", "Two"], "steps": [
      {"id": "q", "kind": "quiz", "title": ["Q", "Q"], "file": ""},
      {"id": "c", "kind": "lesson", "title": ["C", "C"], "file": "c.md"},
      {"id": "x", "kind": "exam", "title": ["X", "X"], "file": ""}]}]
}"#;

const INDEX: &str = r#"{
  "format": "jfn-academy", "version": 1,
  "courses": [{"id": "netz", "title": ["Netz", "Network"], "summary": ["Kurz", "Short"], "level": "beginner", "minutes": 30, "version": 2,
    "files": [{"path": "course.json", "size": 120, "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"}]}]
}"#;

pub(crate) fn run(t: &mut T) {
    files_t(t);
    paths_t(t);
    problems_t(t);
    refs_t(t);
    labs_t(t);
    progress_t(t);
    order_t(t);
    place_t(t);
    page_t(t);
    courses_t(t);
}

fn course() -> Course {
    Course::from_text(COURSE).unwrap_or_default()
}

fn texts(v: &[Problem]) -> Vec<String> {
    v.iter().map(|p| p.msg.text()).collect()
}

fn fatal(v: &[Problem]) -> Vec<String> {
    v.iter().filter(|p| p.fatal).map(|p| p.msg.text()).collect()
}

fn notes(v: &[Problem]) -> Vec<String> {
    v.iter().filter(|p| !p.fatal).map(|p| p.msg.text()).collect()
}

fn s(x: &str) -> String {
    String::from(x)
}

// ---------------------------------------------------------------- the files

fn files_t(t: &mut T) {
    let c = course();
    t.eq("the title in both languages", (c.title.get(false), c.title.get(true)), ("Kurs", "Course"));
    t.eq("level and time", (c.level, c.minutes), (Level::Intermediate, 20));
    t.eq("the version of the content", c.course_version, 3);
    t.eq("two units, five steps", (c.units.len(), c.steps().count()), (2, 5));
    t.eq("a step", (c.step_at(1).map(|x| x.id.as_str()), c.step_at(1).and_then(Step::kind)), (Some("b"), Some(StepKind::Lab)));
    t.eq("a good course has no problems", texts(&c.issues()), Vec::<String>::new());
    // defaults for what is missing
    let d = Course::from_text(r#"{"format": "jfn-course", "version": 1, "title": "Nur Deutsch"}"#).unwrap_or_default();
    t.eq("missing members", (d.title.get(true), d.level, d.minutes, d.units.len()), ("Nur Deutsch", Level::Beginner, 0, 0));
    t.eq("a byte order mark is no error", Course::from_text("\u{FEFF}{\"format\": \"jfn-course\", \"version\": 1}").is_ok(), true);
    // refused, with the message of the project file
    let text = |r: Result<Course, jfn_model::Msg>| r.err().map(|m| m.text()).unwrap_or_default();
    t.eq("a newer version", text(Course::from_text(r#"{"format": "jfn-course", "version": 2}"#)), s("Die Datei hat die Version 2; dieses Programm liest Version 1 bis 1."));
    t.eq("no version", text(Course::from_text(r#"{"format": "jfn-course"}"#)), s("Die Datei hat die Version 0; dieses Programm liest Version 1 bis 1."));
    t.eq("another format", text(Course::from_text(r#"{"format": "jfn", "version": 1}"#)), s("Das ist keine Kursdatei."));
    t.check("no JSON", text(Course::from_text("{ kein json")).starts_with("Die Datei ist kein gültiges JSON"));
    t.eq("no object", text(Course::from_text("[1, 2]")), s("Die Datei enthält keinen Kurs."));
    // the index AK3 reads: its fields, its checks
    let i = Index::from_text(INDEX).unwrap_or_default();
    t.eq("the index's course", (i.courses.len(), i.courses[0].id.as_str(), i.courses[0].version, i.courses[0].minutes), (1, "netz", 2, 30));
    t.eq("its file", (i.courses[0].files[0].path.as_str(), i.courses[0].files[0].size), ("course.json", 120));
    t.eq("a good index", texts(&i.issues()), Vec::<String>::new());
    let itext = |r: Result<Index, jfn_model::Msg>| r.err().map(|m| m.text()).unwrap_or_default();
    t.eq("a newer index", itext(Index::from_text(r#"{"format": "jfn-academy", "version": 9}"#)), s("Die Datei hat die Version 9; dieses Programm liest Version 1 bis 1."));
    t.eq("no index", itext(Index::from_text(r#"{"format": "x", "version": 1}"#)), s("Das ist keine Kursliste."));
    let mut bad = i.clone();
    bad.courses[0].files[0].path = s("../x");
    bad.courses[0].files[0].sha256 = s("abc");
    bad.courses.push(bad.courses[0].clone());
    bad.courses[1].id = s("Netz");
    let found = fatal(&bad.issues());
    t.check("a path out of the course", found.iter().any(|x| x.contains("„../x“ führt mit „..“")));
    t.check("a checksum that is none", found.iter().any(|x| x.contains("ist kein SHA-256")));
    t.check("an id that is none", found.iter().any(|x| x.contains("Die Kennung „Netz“")));
    let mut twice = i.clone();
    twice.courses.push(twice.courses[0].clone());
    t.eq("an id twice", fatal(&twice.issues()), alloc::vec![s("Die Kennung netz kommt doppelt vor.")]);
}

fn paths_t(t: &mut T) {
    for good in ["a.md", "de/01-netz.md", "img/x_y.svg", "labs/a.b.jfnet", "a/b/c/d.png", "Grossbuchstaben.PNG"] {
        t.check(&format!("{} is fine", good), safe_path(good).is_ok());
    }
    let long = "a/".repeat(101) + "b";
    let bad = ["", "/etc/passwd", "\\x", "C:/x", "c:x", "\\\\srv\\x", "https://x/y", "../x", "a/../b", "a/..", "..", "a\\..\\b", "a\\b", "a//b", "a/", "./a", "a/./b", ".hidden/a", "a/.b", "a b", "%2e%2e/x", "a%2fb", "ä.md", "a?b", "a*b", "a.", "nul", "NUL.txt", "a/con.md", "img/com1.png", "lpt9", &long];
    for b in bad {
        t.check(&format!("{:?} is refused", b), safe_path(b).is_err());
    }
    t.eq("what it says about ..", safe_path("a/../b").err().map(|m| m.text()), Some(s("Der Pfad „a/../b“ führt mit „..“ aus dem Ordner des Kurses.")));
    t.eq("what it says about /", safe_path("/x").err().map(|m| m.text()), Some(s("Der Pfad „/x“ ist absolut; erlaubt sind nur Pfade im Ordner des Kurses.")));
    for good in ["a", "netz-1", "a_b", "01-anfang"] {
        t.check(&format!("id {}", good), safe_id(good));
    }
    for b in ["", "A", "a b", "-a", "_a", "a/b", "a.b", "ä", "nul", "com1"] {
        t.check(&format!("no id {:?}", b), !safe_id(b));
    }
    t.check("a long id", !safe_id(&"a".repeat(65)));
    // what a picture's address may name: the view asks, this decides
    let base = "data://academy/k/";
    t.eq("a picture", image_file(base, "data://academy/k/img/x.svg"), Some("img/x.svg"));
    t.eq("a photo", image_file(base, "data://academy/k/a.JPG"), Some("a.JPG"));
    for addr in ["data://academy/k/../x.png", "data://academy/k/a/../../x.png", "data://academy/other/x.png", "data://academy/k/x.exe", "data://academy/k/x", "https://example.org/x.png", "data:image/png;base64,AAAA", "/etc/x.png", "C:/x.png", "img/x.png", "data://academy/k/"] {
        t.eq(&format!("not {}", addr), image_file(base, addr), None);
    }
    // the lesson's files: the language asked for first; an unknown one takes the source's
    t.eq("German first", lesson_files("a.md", "de"), [(s("de/a.md"), "de"), (s("en/a.md"), "en")]);
    t.eq("English first", lesson_files("a.md", "en"), [(s("en/a.md"), "en"), (s("de/a.md"), "de")]);
    t.eq("another language", lesson_files("a.md", "fr")[0].1, "de");
}

fn problems_t(t: &mut T) {
    let with = |f: &dyn Fn(&mut Course)| {
        let mut c = course();
        f(&mut c);
        c
    };
    // an unknown kind is an error of the course, with the kinds there are
    let c = with(&|c| c.units[0].steps[0].kind = s("vortrag"));
    t.eq("an unknown step kind", fatal(&c.issues()), alloc::vec![s("Schritt 1: die Art „vortrag“ gibt es nicht (lesson, lab, quiz, fault, exam).")]);
    t.eq("it has no kind", c.step_at(0).and_then(Step::kind), None);
    t.check("it cannot be opened", !c.step_at(0).is_some_and(Step::ready));
    // every kind can be opened; a quiz, a test or a fault hunt without a file is only announced
    t.check("a quiz is known", course().step_at(2).and_then(Step::kind) == Some(StepKind::Quiz));
    t.check("a quiz without a file waits", !course().step_at(2).is_some_and(Step::ready));
    t.eq("every kind is ready", StepKind::ALL.iter().filter(|k| k.ready()).count(), 5);
    // a bad path gives its message, with the step
    let c = with(&|c| c.units[0].steps[0].file = s("../../geheim.md"));
    t.eq("a bad path", fatal(&c.issues()), alloc::vec![s("Schritt 1: Der Pfad „../../geheim.md“ führt mit „..“ aus dem Ordner des Kurses.")]);
    t.eq("no file is looked for there", texts(&c.file_issues(&|_| false)).iter().filter(|x| x.contains("geheim")).count(), 0);
    let c = with(&|c| c.units[1].steps[0].file = s("/absolut.json"));
    t.eq("a quiz's path too", fatal(&c.issues()), alloc::vec![s("Schritt 3: Der Pfad „/absolut.json“ ist absolut; erlaubt sind nur Pfade im Ordner des Kurses.")]);
    let c = with(&|c| c.units[1].steps[0].file = s("quiz/q.txt"));
    t.eq("a quiz's ending", fatal(&c.issues()), alloc::vec![s("Schritt 3: die Datei quiz/q.txt endet nicht auf .json.")]);
    let c = with(&|c| c.units[1].steps[0].file = s("quiz/q.json"));
    t.eq("a quiz's file missing", fatal(&c.file_issues(&|_| false)).iter().filter(|x| x.contains("quiz/q.json")).count(), 1);
    let c = with(&|c| c.units[0].steps[0].file = s(""));
    t.eq("a lesson without a file", fatal(&c.issues()), alloc::vec![s("Schritt 1: Der Pfad ist leer.")]);
    let c = with(&|c| c.units[0].steps[1].file = s("labs/b.txt"));
    t.eq("the wrong ending", fatal(&c.issues()), alloc::vec![s("Schritt 2: die Datei labs/b.txt endet nicht auf .jfnet.")]);
    // ids and titles
    let c = with(&|c| c.units[0].steps[1].id = s("a"));
    t.eq("an id twice", fatal(&c.issues()), alloc::vec![s("Die Kennung a kommt doppelt vor.")]);
    let c = with(&|c| c.units[0].steps[0].id = s("A B"));
    t.eq("a bad id", fatal(&c.issues()), alloc::vec![s("Schritt 1: die Kennung „A B“ darf nur aus kleinen Buchstaben, Ziffern, - und _ bestehen.")]);
    let c = with(&|c| c.units[0].steps[0].title = jfn_model::Text::default());
    t.eq("a step without a title", fatal(&c.issues()), alloc::vec![s("Schritt 1 hat keinen Titel.")]);
    let c = with(&|c| c.title = jfn_model::Text::default());
    t.eq("a course without a title", fatal(&c.issues()), alloc::vec![s("Der Kurs hat keinen Titel.")]);
    let c = with(&|c| c.units.clear());
    t.check("a course without steps", fatal(&c.issues()).contains(&s("Der Kurs hat keine Schritte.")));
    let c = with(&|c| {
        let step = c.units[0].steps[0].clone();
        c.units[0].steps = (0..1001).map(|k| Step { id: format!("s{}", k), ..step.clone() }).collect();
    });
    t.check("a course with too many steps", fatal(&c.issues()).contains(&s("Der Kurs hat mehr als 1000 Schritte.")));
    let c = with(&|c| {
        c.title.en = String::new();
        c.units[0].steps[0].title.en = String::new();
        c.course_version = 0;
    });
    t.eq("no English title is a note", (fatal(&c.issues()).len(), notes(&c.issues()).len()), (0, 3));
    // a lesson in one language: a note, the other shows; in none: an error; a lab's file too
    let c = course();
    let all = |p: &str| matches!(p, "de/a.md" | "en/a.md" | "de/c.md" | "en/c.md" | "labs/b.jfnet");
    t.eq("every file there", texts(&c.file_issues(&all)), Vec::<String>::new());
    let no_en = |p: &str| all(p) && p != "en/c.md";
    t.eq("a lesson missing in one language", (fatal(&c.file_issues(&no_en)), notes(&c.file_issues(&no_en))), (Vec::new(), alloc::vec![s("Schritt c: die Lektion c.md fehlt im Ordner en; es erscheint die andere Sprache.")]));
    let none = |p: &str| all(p) && !p.ends_with("c.md");
    t.eq("a lesson missing in both", fatal(&c.file_issues(&none)), alloc::vec![s("Schritt c: die Lektion c.md fehlt in jeder Sprache.")]);
    let no_lab = |p: &str| all(p) && p != "labs/b.jfnet";
    t.eq("a lab missing", fatal(&c.file_issues(&no_lab)), alloc::vec![s("Schritt b: die Datei labs/b.jfnet fehlt.")]);
    // a folder as a whole
    let (got, v) = load("k", Some(COURSE), &all);
    t.check("a folder that is fine", got.is_some() && v.is_empty());
    let (got, v) = load("k", None, &all);
    t.eq("no course.json", (got.is_none(), fatal(&v)), (true, alloc::vec![s("Die Datei course.json fehlt oder lässt sich nicht lesen.")]));
    let (got, v) = load("k", Some(r#"{"format": "jfn-course", "version": 99}"#), &all);
    t.eq("a course too new", (got.is_none(), fatal(&v)), (true, alloc::vec![s("Die Datei hat die Version 99; dieses Programm liest Version 1 bis 1.")]));
    let (_, v) = load("Mein Kurs", Some(COURSE), &all);
    t.eq("a folder's name that is no id", fatal(&v), alloc::vec![s("Die Kennung „Mein Kurs“ darf nur aus kleinen Buchstaben, Ziffern, - und _ bestehen.")]);
}

fn refs_t(t: &mut T) {
    let md = "# Titel\n\nEin [Link](step:a \"mit Titel\") und ein ![Bild](img/a.png) und `![kein](img/b.png)`.\n\n```\n[kein Link](step:b)\n```\n\n[![Bild im Link](img/c.png)](lab:b) und <https://example.org/> und \\[kein](x).\n\n[Web](https://example.org/a?b=c) [leer]() [<spitz>](<step:c>)\n";
    let refs = lesson_refs(md);
    let want = [(false, "step:a"), (true, "img/a.png"), (true, "img/c.png"), (false, "lab:b"), (false, "https://example.org/a?b=c"), (false, ""), (false, "step:c")];
    t.eq("the references", refs.iter().map(|r| (r.image, r.target.as_str())).collect::<Vec<_>>(), want.to_vec());
    // the same as the reader the app shows it with, for what an author writes
    let simple = "Ein [Link](step:a) und ein ![Bild](img/a.png).\n\n```\n![kein](img/b.png)\n```\n\n[Web](https://example.org/a) ![Zwei](img/c.svg)\n";
    let mut st = MdViewState::new();
    st.set_markdown(simple);
    let ours = lesson_refs(simple);
    let images: Vec<&str> = ours.iter().filter(|r| r.image).map(|r| r.target.as_str()).collect();
    let links: Vec<&str> = ours.iter().filter(|r| !r.image).map(|r| r.target.as_str()).collect();
    t.eq("the reader's images", st.doc().images().iter().map(|m| m.src.as_str()).collect::<Vec<_>>(), images);
    t.eq("the reader's links", st.doc().links().iter().map(String::as_str).collect::<Vec<_>>(), links);
    // where a link leads
    t.eq("step:", link_to("step:a"), LinkTo::Step("a"));
    t.eq("lab:", link_to("lab:b"), LinkTo::Lab("b"));
    t.eq("the web", (link_to("https://x"), link_to("HTTP://x")), (LinkTo::Web, LinkTo::Web));
    for other in ["", "a.md", "../a", "mailto:a@b", "javascript:x", "file:///c:/x", "#oben"] {
        t.eq(other, link_to(other), LinkTo::Other);
    }
    // what a lesson refers to, against its course
    let c = course();
    let there = |p: &str| p == "img/a.png";
    let v = c.lesson_issues("a", "![a](img/a.png) [b](step:b) [c](lab:b) [d](https://example.org/)", &there);
    t.eq("a lesson that is fine", texts(&v), Vec::<String>::new());
    let v = c.lesson_issues("a", "![a](img/fehlt.png) ![b](../b.png) [c](step:weg) [d](lab:a) [e](a.md)", &there);
    t.eq(
        "a lesson with mistakes",
        texts(&v),
        alloc::vec![
            s("Schritt a: das Bild img/fehlt.png fehlt."),
            s("Schritt a: Der Pfad „../b.png“ führt mit „..“ aus dem Ordner des Kurses."),
            s("Schritt a: den Schritt weg gibt es nicht."),
            s("Schritt a: a ist kein Labor."),
            s("Schritt a: der Link „a.md“ führt nirgendwohin; es gibt step:, lab:, http: und https:."),
        ],
    );
    t.check("none of them keeps the course from being offered", v.iter().all(|p| !p.fatal));
}

fn labs_t(t: &mut T) {
    t.check("no project", lab_issues("b", "{ kein json").iter().all(|p| p.fatal && p.msg.text().starts_with("Schritt b: Die Datei ist kein gültiges JSON")));
    let plain = Project::new("Frei", 1).to_text();
    t.eq("a project without a task", fatal(&lab_issues("b", &plain)), alloc::vec![s("Schritt b: die Datei hat keine Aufgabe.")]);
    let mut p = Project::new("Lab", 1);
    p.lab = Some(jfn_model::Lab { title: jfn_model::Text::new("T", "T"), ..Default::default() });
    let found = fatal(&lab_issues("b", &p.to_text()));
    t.check("a lab without goals", found.iter().any(|x| x.contains("an denen sich sein Ende erkennen lässt")));
}

// ---------------------------------------------------------------- the progress

fn progress_t(t: &mut T) {
    let mut p = Progress::default();
    t.check("nothing done", !p.is_done("k", "a"));
    t.check("done now", p.mark_done("k", "a", 1_700_000_000));
    t.check("done", p.is_done("k", "a"));
    t.check("once", !p.mark_done("k", "a", 1_800_000_000));
    t.eq("the first time counts", p.courses["k"].steps["a"].when, 1_700_000_000);
    t.check("another course knows nothing of it", !p.is_done("l", "a"));
    let text = p.to_text();
    t.check("its format", text.contains("\"format\": \"jfn-progress\"") && text.contains("\"version\": 1"));
    t.check("the state in words", text.contains("\"state\": \"done\""));
    let back = Progress::from_text(&text);
    t.check("round trip", back.is_done("k", "a"));
    t.eq("round trip: the record", back.courses["k"].steps["a"].clone(), StepRecord { state: StepMark::Done, when: 1_700_000_000, best: 0, tries: 0 });
    t.eq("written the same", back.to_text(), text);
    // corrupt, missing, of another kind or newer: empty, never a crash
    for bad in ["", "{", "kein json", "[1,2,3]", "null", "42", "{\"format\": \"jfn\", \"version\": 1}", "{\"version\": 1}", "{\"format\": \"jfn-progress\", \"version\": 9, \"courses\": {\"k\": {}}}", "\u{0}\u{1}\u{2}"] {
        t.eq(&format!("{:?} is empty", bad), Progress::from_text(bad).courses.len(), 0);
    }
    let odd = Progress::from_text(r#"{"format": "jfn-progress", "version": 1, "courses": {"k": {"steps": {"a": {"state": "wer weiß"}, "b": 7, "c": {"state": "done"}}}, "l": 3}}"#);
    t.check("what it cannot read counts as open", !odd.is_done("k", "a") && !odd.is_done("k", "b") && !odd.is_done("l", "a"));
    t.check("what it can read stays", odd.is_done("k", "c"));
    // a new version of a course keeps it, as long as the ids stay; a step that went keeps its entry
    let old = course();
    let mut q = Progress::default();
    q.mark_done("k", "a", 1);
    q.mark_done("k", "c", 2);
    let mut new = course();
    new.course_version = 4;
    new.units[0].steps.remove(0);
    new.units[1].steps.insert(0, Step { id: s("neu"), kind: s("lesson"), title: jfn_model::Text::new("N", "N"), file: s("n.md"), ..Step::default() });
    let q2 = Progress::from_text(&q.to_text());
    t.eq("c stays done in the new version", q2.done_count(&new, "k"), 1);
    t.eq("the step that went keeps its entry", q2.is_done("k", "a"), true);
    t.eq("and counts when it comes back", q2.done_count(&old, "k"), 2);
    // reset: this course alone
    q.mark_done("l", "x", 3);
    t.check("reset", q.clear("k"));
    t.check("nothing left of it", !q.is_done("k", "a") && !q.is_done("k", "c"));
    t.check("the other course stays", q.is_done("l", "x"));
    t.check("nothing to reset", !q.clear("k"));
    // through its file; the test's own folder, never the user's
    let path = "mem://jfn-selftest/academy/progress.json";
    t.eq("no file yet", store::read_progress(path).courses.len(), 0);
    t.check("written", store::write_progress(path, &q));
    t.check("read back", store::read_progress(path).is_done("l", "x"));
    plat::write_file(path, b"{ \"format\": kaputt");
    t.eq("a corrupt file", store::read_progress(path).courses.len(), 0);
}

fn order_t(t: &mut T) {
    let c = course();
    use StepState::*;
    let mut p = Progress::default();
    t.eq("at the start", p.states(&c, "k"), alloc::vec![Current, Open, NotYet, Open, NotYet]);
    t.eq("what can be done", c.ready_count(), 3);
    t.eq("nothing done", p.done_count(&c, "k"), 0);
    t.eq("Weiter starts at the first", p.next_open(&c, "k"), Some(0));
    p.mark_done("k", "a", 1);
    t.eq("one done", p.states(&c, "k"), alloc::vec![Done, Current, NotYet, Open, NotYet]);
    t.eq("Weiter goes to the next", p.next_open(&c, "k"), Some(1));
    // a lab left open stays where Weiter goes, even with what follows done
    p.mark_done("k", "c", 2);
    t.eq("a lab left open", p.states(&c, "k"), alloc::vec![Done, Current, NotYet, Done, NotYet]);
    t.eq("its place", p.next_open(&c, "k"), Some(1));
    t.eq("done of what can be done", p.done_count(&c, "k"), 2);
    p.mark_done("k", "b", 3);
    t.eq("all that can be done", p.states(&c, "k"), alloc::vec![Done, Done, NotYet, Done, NotYet]);
    t.eq("nothing left", p.next_open(&c, "k"), None);
    t.eq("all of them", p.done_count(&c, "k"), 3);
    // the order, past what cannot be opened yet
    t.eq("the next from a", c.next_ready(0), Some(1));
    t.eq("over the quiz", c.next_ready(1), Some(3));
    t.eq("after the last", c.next_ready(3), None);
    t.eq("the one before c, over the quiz", c.prev_ready(3), Some(1));
    t.eq("before the first", c.prev_ready(0), None);
    t.eq("by id", (c.index_of("c"), c.index_of("weg")), (Some(3), None));
}

// ---------------------------------------------------------------- what a run looks at

fn place_t(t: &mut T) {
    t.eq("a window: the app's data", store::place(false, None), Place { courses: Some(s("data://academy")), progress: s("data://academy/progress.json") });
    t.eq("a window with a folder: it holds the progress too", store::place(false, Some(s("D:\\kurse\\"))), Place { courses: Some(s("D:/kurse")), progress: s("D:/kurse/progress.json") });
    t.eq("a test: no courses, nothing of the user's", store::place(true, None), Place { courses: None, progress: s("mem://jfn-academy/progress.json") });
    t.eq("a test with a folder: the progress stays in memory", store::place(true, Some(s("apps/JFNetworker/academy"))), Place { courses: Some(s("apps/JFNetworker/academy")), progress: s("mem://jfn-academy/progress.json") });
}

fn page_t(t: &mut T) {
    let root = "mem://jfn-selftest/academy-page";
    let put = |rel: &str, text: &str| {
        plat::write_file(&format!("{}/{}", root, rel), text.as_bytes());
    };
    let good = |title: &str| format!(r#"{{"format": "jfn-course", "version": 1, "course_version": 1, "title": ["{}", "{} en"], "summary": ["S", "S en"], "minutes": 5, "units": [{{"title": ["U", "U"], "steps": [{{"id": "a", "kind": "lesson", "title": ["A", "A en"], "file": "a.md"}}, {{"id": "b", "kind": "lesson", "title": ["B", "B en"], "file": "b.md"}}]}}]}}"#, title, title);
    put("gut/course.json", &good("Gut"));
    put("gut/de/a.md", "# A deutsch");
    put("gut/en/a.md", "# A english");
    put("gut/de/b.md", "\u{FEFF}# B nur deutsch");
    put("gut/img/x.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"/>");
    put("gut/geheim.exe", "MZ");
    put("zu-neu/course.json", r#"{"format": "jfn-course", "version": 7, "title": "Neu"}"#);
    put("kaputt/course.json", "{ kein json");
    put("pfad/course.json", r#"{"format": "jfn-course", "version": 1, "title": ["Pfad", "Path"], "units": [{"title": ["U", "U"], "steps": [{"id": "a", "kind": "lesson", "title": ["A", "A"], "file": "../../a.md"}]}]}"#);
    put("ohne-datei/notiz.txt", "x");
    put(".halbfertig/course.json", &good("Unfertig"));
    put("progress.json", "{}");
    let list = store::list(root);
    t.eq("every folder, by name, and no hidden one", list.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), alloc::vec!["gut", "kaputt", "ohne-datei", "pfad", "zu-neu"]);
    let by = |id: &str| list.iter().find(|e| e.id == id);
    t.check("the good course is offered", by("gut").is_some_and(|e| e.usable().is_some()));
    for id in ["kaputt", "ohne-datei", "pfad", "zu-neu"] {
        t.check(&format!("{} is not", id), by(id).is_some_and(|e| e.usable().is_none() && e.problems.iter().any(|p| p.fatal)));
    }
    t.eq("its message", by("zu-neu").map(|e| fatal(&e.problems)), Some(alloc::vec![s("Die Datei hat die Version 7; dieses Programm liest Version 1 bis 1.")]));
    t.eq("a title it has", by("pfad").map(|e| e.title(true)), Some("Path"));
    t.eq("else the folder's name", by("kaputt").map(|e| e.title(false)), Some("kaputt"));
    // a lesson: the language asked for, else the other; a mark in front of the text is not part of it
    let dir = format!("{}/gut", root);
    t.eq("German", store::lesson(&dir, "a.md", "de"), Some((s("# A deutsch"), "de")));
    t.eq("English", store::lesson(&dir, "a.md", "en"), Some((s("# A english"), "en")));
    t.eq("English missing: German", store::lesson(&dir, "b.md", "en"), Some((s("# B nur deutsch"), "de")));
    t.eq("missing in both", store::lesson(&dir, "c.md", "de"), None);
    // a picture only from the course's folder, only as a picture
    let base = format!("{}/", dir);
    t.check("a picture of the course", store::image(&dir, &format!("{}img/x.svg", base)).is_some());
    t.eq("a picture that is not there", store::image(&dir, &format!("{}img/y.svg", base)), None);
    t.eq("not a program", store::image(&dir, &format!("{}geheim.exe", base)), None);
    t.eq("not out of the folder", store::image(&dir, &format!("{}../kaputt/course.json", base)), None);
    t.eq("not another course", store::image(&dir, &format!("{}/kaputt/x.svg", root)), None);
    t.eq("not the web", store::image(&dir, "https://example.org/x.svg"), None);
    // the cards: one per folder, the bad one says so, the progress shows
    let mut a = Academy::new();
    a.setup(true, Some(s(root)));
    a.ensure();
    let cards = a.cards();
    t.eq("as many cards as folders", cards.len(), 5);
    t.eq("a course's card", cards[0].clone(), Card { title: s("Gut"), text: s("Einsteiger · 5 Min. · S"), badge: None });
    t.eq("a bad course's card", cards[1].badge.clone(), Some((ui::BADGE_WARN, s("Fehlerhaft"))));
    a.progress.mark_done("gut", "a", 1);
    t.eq("how far", a.cards()[0].badge.clone(), Some((ui::BADGE_INFO, s("1 von 2"))));
    a.progress.mark_done("gut", "b", 2);
    t.eq("all of it", a.cards()[0].badge.clone(), Some((ui::BADGE_OK, s("Abgeschlossen"))));
    // jfn-sim course: what it says about these folders, with the code of its last line
    let (lines, errors) = jfn_run::check_course(&format!("{}/pfad", root));
    t.check("the course with the bad path", errors > 0 && lines.iter().any(|l| l.starts_with("Fehler: Schritt 1: Der Pfad „../../a.md“")));
    // ... and it reads nothing outside the folder: this file would add a line about itself
    put("draussen.jfnet", "{}");
    put("lab-hinaus/course.json", r#"{"format": "jfn-course", "version": 1, "course_version": 1, "title": ["L", "L"], "summary": ["S", "S"], "units": [{"title": ["U", "U"], "steps": [{"id": "a", "kind": "lab", "title": ["A", "A"], "file": "../draussen.jfnet"}]}]}"#);
    let (lines, _) = jfn_run::check_course(&format!("{}/lab-hinaus", root));
    t.eq("no file behind a refused path", lines, alloc::vec![s("Fehler: Schritt 1: Der Pfad „../draussen.jfnet“ führt mit „..“ aus dem Ordner des Kurses.\n")]);
    let (lines, errors) = jfn_run::check_course(&format!("{}/zu-neu", root));
    t.eq("the course too new", (lines, errors), (alloc::vec![s("Fehler: Die Datei hat die Version 7; dieses Programm liest Version 1 bis 1.\n")], 1));
    let (lines, errors) = jfn_run::check_course(&format!("{}/gut", root));
    t.check("the good one: notes, no errors", errors == 0 && lines.iter().any(|l| l.starts_with("Hinweis: Schritt b: die Lektion b.md fehlt im Ordner en")));
    // a headless run without a folder offers nothing
    let mut none = Academy::new();
    none.setup(true, None);
    none.ensure();
    t.eq("no folder, no courses", none.cards().len(), 0);
}

// ---------------------------------------------------------------- the courses of the repository

fn courses_t(t: &mut T) {
    // the run names the folder (tests/model.uis): that also grants it
    let Some(root) = plat::arg_value("--academy-dir") else { return };
    let list = store::list(&root);
    t.check(&format!("courses in {}", root), !list.is_empty());
    t.check("the sample is one of them", list.iter().any(|e| e.id == "beispiel"));
    for e in &list {
        // jfn-sim course: the file, the lessons with their pictures and links, the labs with `check`
        let (lines, errors) = jfn_run::check_course(&e.dir);
        t.eq(&format!("{}: nothing to say about it", e.id), lines, Vec::<String>::new());
        t.eq(&format!("{}: no errors", e.id), errors, 0);
        let Some(c) = e.usable() else {
            t.check(&format!("{} is offered", e.id), false);
            continue;
        };
        t.check(&format!("{}: an id of its own", e.id), safe_id(&e.id));
        t.check(&format!("{}: a summary in both languages", e.id), !c.summary.is_empty() && !c.summary.en.is_empty());
        t.check(&format!("{}: it says how long it takes", e.id), c.minutes > 0);
        for step in c.steps() {
            let name = format!("{} {}", e.id, step.id);
            match step.kind() {
                Some(StepKind::Lesson) => lesson_t(t, &name, e, step),
                Some(StepKind::Lab) => lab_t(t, &name, e, step),
                _ => {}
            }
        }
    }
}

// a lesson of a course: in both languages, not the same text, a picture that decodes, our own words
fn lesson_t(t: &mut T, name: &str, e: &store::Entry, step: &Step) {
    let de = store::lesson(&e.dir, &step.file, "de");
    let en = store::lesson(&e.dir, &step.file, "en");
    t.eq(&format!("{}: German", name), de.as_ref().map(|x| x.1), Some("de"));
    t.eq(&format!("{}: English", name), en.as_ref().map(|x| x.1), Some("en"));
    let (Some((de, _)), Some((en, _))) = (de, en) else { return };
    t.check(&format!("{}: two texts", name), de != en);
    for text in [&de, &en] {
        name_rule(t, name, text);
        for r in lesson_refs(text).iter().filter(|r| r.image) {
            let data = store::image(&e.dir, &format!("{}/{}", e.dir, r.target));
            t.check(&format!("{}: {} can be read", name, r.target), data.is_some());
            t.check(&format!("{}: {} is a picture", name, r.target), data.is_some_and(|d| img_svg::Svg::parse(&d).is_ok() || img_core::io::decode(&d).is_ok()));
        }
    }
}

// a lab of a course: no goal reached at the start, every one with its solution; both languages
fn lab_t(t: &mut T, name: &str, e: &store::Entry, step: &Step) {
    let path = format!("{}/{}", e.dir, step.file);
    let Ok(p) = crate::doc::file::read(&path) else {
        t.check(&format!("{} loads", name), false);
        return;
    };
    let Some(lab) = p.lab.clone() else {
        t.check(&format!("{} has a task", name), false);
        return;
    };
    t.check(&format!("{}: a title in both languages", name), !lab.title.is_empty() && !lab.title.en.is_empty());
    for g in &lab.goals {
        t.check(&format!("{}: goal {} in both languages", name, g.text.de), !g.text.en.is_empty());
    }
    name_rule(t, name, &p.to_text());
    let reached = |p: &Project| {
        let mut sim = Sim::new(p);
        let mut run = LabRun::new(&lab);
        for _ in 0..6 {
            let _ = sim.send(Cmd::RunFor(SEC));
            sim.take_reports();
            run.eval(p, sim.engine(), &|_| String::new());
        }
        run.reached()
    };
    t.eq(&format!("{}: no goal at the start", name), reached(&p), 0);
    // the sample's solution; every other course's labs are solved at the console in coursetest.rs
    if e.id == "beispiel" && step.id == "erstes-netz" {
        let mut solved = p.clone();
        if let Some(d) = solved.devices.iter_mut().find(|d| d.name == "PC2") {
            d.startup_config.push_str("ip addr add 10.0.0.2/24 dev eth0\n");
        }
        t.eq(&format!("{}: every goal with the address", name), reached(&solved), lab.goals.len());
    }
}

// the naming rule of the README: our own words, no one else's names
fn name_rule(t: &mut T, name: &str, text: &str) {
    let low = text.to_lowercase();
    for word in ["networking academy", "netacad", "cisco", "packet tracer"] {
        t.check(&format!("{}: no {}", name, word), !low.contains(word));
    }
}
