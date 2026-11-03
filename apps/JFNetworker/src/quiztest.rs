// quiztest -- quizzes, tests and fault hunts of the Akademie without a window (part of --selftest, group
// "quiz"): the format and what is wrong with a file, grading of every kind, partial credit, patterns, the
// tolerance of a number, the order of an attempt from its seed, attempts and the time limit, what goes into
// the progress, a step's `requires`; a lab's start faults and locks and their defaults (a lab without them
// stays as it was), a diagnosis, the check that something must fail, determinism and replay with start
// faults; and the sample course's quiz, test and fault hunt: without its fix the result is not full, with it
// it is.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::academy::{fault_issues, quiz_issues, Course, Problem, Progress, Step, StepKind, StepState};
use jfn_model::lab::{Assess, Feedback, Item, Lab, Lock, Target, Text};
use jfn_model::project::{Link, PortRef, Project};
use jfn_model::quiz::{fixed, Attempt, Marked, QKind, Question, Quiz, Reply, MILLI};
use jfn_model::scenario::{self, FaultSpec};
use jfn_sim::assess::{item_text, pick_right, pick_text, Grader, Pick};
use jfn_sim::goal::judge;
use jfn_sim::keyframe::digest;
use jfn_sim::{Action, Budget, Cmd, Engine, Fault, Inline, SEC};

use crate::doc::sim::Sim;
use crate::selftest::T;
use crate::workspace::academy::store;
use crate::workspace::assess::Run;

pub(crate) fn run(t: &mut T) {
    format_t(t);
    issues_t(t);
    grade_t(t);
    plan_t(t);
    attempt_t(t);
    progress_t(t);
    requires_t(t);
    lab_t(t);
    start_t(t);
    diagnosis_t(t);
    negative_t(t);
    sample_t(t);
}

fn s(x: &str) -> String {
    String::from(x)
}

fn fatal(v: &[Problem]) -> Vec<String> {
    v.iter().filter(|p| p.fatal).map(|p| p.msg.text()).collect()
}

fn notes(v: &[Problem]) -> Vec<String> {
    v.iter().filter(|p| !p.fatal).map(|p| p.msg.text()).collect()
}

const GOOD: &str = r#"{
  "format": "jfn-quiz", "version": 1, "morgen": 1,
  "title": ["Titel", "Title"], "pass": 60, "minutes": 2, "attempts": 2, "shuffle": true,
  "questions": [
    {"kind": "single", "stem": ["Eins?", "One?"], "points": 2, "explain": ["Darum.", "Because."],
     "answers": [{"text": ["A", "A"]}, {"text": ["B", "B"], "right": true, "explain": ["Ja", "Yes"]}, {"text": ["C", "C"]}]},
    {"kind": "multi", "stem": ["Zwei?", "Two?"], "explain": ["Darum.", "Because."],
     "answers": [{"text": ["A", "A"], "right": true}, {"text": ["B", "B"], "right": true}, {"text": ["C", "C"]}, {"text": ["D", "D"]}]},
    {"kind": "text", "stem": ["Drei?", "Three?"], "explain": ["Darum.", "Because."], "accept": ["10\\.0\\.0\\.0 ?/ ?24", "netz( eins)?"]},
    {"kind": "number", "stem": ["Vier?", "Four?"], "explain": ["Darum.", "Because."], "value": 2.5, "tolerance": "0,5"},
    {"kind": "order", "stem": ["Fünf?", "Five?"], "explain": ["Darum.", "Because."], "order": [["a", "a"], ["b", "b"], ["c", "c"]]}
  ]
}"#;

fn good() -> Quiz {
    Quiz::from_text(GOOD).unwrap_or_default()
}

// ---------------------------------------------------------------- the file

fn format_t(t: &mut T) {
    let q = good();
    t.eq("the questions", q.questions.len(), 5);
    t.eq("its figures", (q.pass, q.minutes, q.attempts, q.shuffle), (60, 2, 2, true));
    t.eq("the title", (q.title.get(false), q.title.get(true)), ("Titel", "Title"));
    t.eq("a good quiz has no issues", q.issues(), Vec::<Problem>::new());
    t.eq("the kinds", q.questions.iter().map(Question::kind_of).collect::<Vec<_>>(), [QKind::Single, QKind::Multi, QKind::Text, QKind::Number, QKind::Order].map(Some).to_vec());
    t.eq("what they are worth", q.questions.iter().map(Question::worth).collect::<Vec<_>>(), alloc::vec![2, 1, 1, 1, 1]);
    t.eq("the points in thousandths", q.total(), 6 * MILLI as u64);
    t.eq("what passes", (q.pass_percent(), Quiz::default().pass_percent()), (60, 100));
    let text = |r: Result<Quiz, jfn_model::Msg>| r.err().map(|m| m.text()).unwrap_or_default();
    t.eq("a newer version", text(Quiz::from_text(r#"{"format": "jfn-quiz", "version": 2}"#)), s("Die Datei hat die Version 2; dieses Programm liest Version 1 bis 1."));
    t.eq("another format", text(Quiz::from_text(r#"{"format": "jfn-course", "version": 1}"#)), s("Das ist keine Quizdatei."));
    t.check("no JSON", text(Quiz::from_text("{ kein json")).starts_with("Die Datei ist kein gültiges JSON"));
    t.eq("no object", text(Quiz::from_text("[1]")), s("Die Datei enthält kein Quiz."));
    t.check("a byte order mark is no error", Quiz::from_text("\u{FEFF}{\"format\": \"jfn-quiz\", \"version\": 1}").is_ok());
}

// a quiz with this one question (as JSON)
fn one(q: &str) -> Quiz {
    Quiz::from_text(&format!(r#"{{"format": "jfn-quiz", "version": 1, "questions": [{}]}}"#, q)).unwrap_or_default()
}

fn issues_t(t: &mut T) {
    let ab = r#""answers": [{"text": ["A", "A"], "right": true}, {"text": ["B", "B"]}]"#;
    let stem = r#""stem": ["F", "Q"], "explain": ["E", "E"]"#;
    let bad = |t: &mut T, name: &str, q: String, want: &str| t.eq(name, fatal(&one(&q).issues()), alloc::vec![s(want)]);
    bad(t, "a question without a right answer", format!(r#"{{"kind": "single", {}, "answers": [{{"text": ["A", "A"]}}, {{"text": ["B", "B"]}}]}}"#, stem), "Frage 1 hat keine richtige Antwort.");
    bad(t, "multi without a right answer", format!(r#"{{"kind": "multi", {}, "answers": [{{"text": ["A", "A"]}}, {{"text": ["B", "B"]}}]}}"#, stem), "Frage 1 hat keine richtige Antwort.");
    bad(t, "two right on single", format!(r#"{{"kind": "single", {}, "answers": [{{"text": ["A", "A"], "right": true}}, {{"text": ["B", "B"], "right": true}}]}}"#, stem), "Frage 1 hat 2 richtige Antworten; bei „single“ ist es genau eine.");
    bad(t, "one answer only", format!(r#"{{"kind": "single", {}, "answers": [{{"text": ["A", "A"], "right": true}}]}}"#, stem), "Frage 1 braucht mindestens zwei Antworten.");
    bad(t, "a kind that is none", format!(r#"{{"kind": "ratespiel", {}}}"#, stem), "Frage 1: die Art „ratespiel“ gibt es nicht (single, multi, text, number, order).");
    bad(t, "a pattern that does not compile", format!(r#"{{"kind": "text", {}, "accept": ["ok", "(("]}}"#, stem), "Frage 1: das Muster „((“ lässt sich nicht übersetzen: Unmatched ( or \\(");
    bad(t, "no patterns", format!(r#"{{"kind": "text", {}}}"#, stem), "Frage 1 nennt keine akzeptierten Muster (accept).");
    bad(t, "no value", format!(r#"{{"kind": "number", {}}}"#, stem), "Frage 1: der richtige Wert (value) ist keine Zahl.");
    bad(t, "a value that is none", format!(r#"{{"kind": "number", {}, "value": "viele"}}"#, stem), "Frage 1: der richtige Wert (value) ist keine Zahl.");
    bad(t, "a tolerance below 0", format!(r#"{{"kind": "number", {}, "value": 1, "tolerance": -1}}"#, stem), "Frage 1: die Toleranz (tolerance) ist keine Zahl ab 0.");
    bad(t, "one entry to order", format!(r#"{{"kind": "order", {}, "order": [["a", "a"]]}}"#, stem), "Frage 1 braucht mindestens zwei Einträge zum Ordnen.");
    bad(t, "no points", format!(r#"{{"kind": "single", {}, "points": 0, {}}}"#, stem, ab), "Frage 1 ist 0 Punkte wert.");
    // a language missing, wherever it is
    bad(t, "the stem without English", format!(r#"{{"kind": "single", "stem": "Nur Deutsch", "explain": ["E", "E"], {}}}"#, ab), "Frage 1: der Text fehlt auf Englisch.");
    bad(t, "the stem without German", format!(r#"{{"kind": "single", "stem": {{"en": "English only"}}, "explain": ["E", "E"], {}}}"#, ab), "Frage 1: der Text fehlt auf Deutsch.");
    bad(t, "no stem", format!(r#"{{"kind": "single", "explain": ["E", "E"], {}}}"#, ab), "Frage 1: der Text fehlt auf Deutsch.");
    bad(t, "the explanation without English", format!(r#"{{"kind": "single", "stem": ["F", "Q"], "explain": "Nur Deutsch", {}}}"#, ab), "Frage 1: die Erklärung fehlt auf Englisch.");
    bad(t, "an answer without English", format!(r#"{{"kind": "single", {}, "answers": [{{"text": ["A", "A"], "right": true}}, {{"text": "B"}}]}}"#, stem), "Frage 1, Antwort 2: der Text fehlt auf Englisch.");
    bad(t, "an answer's explanation", format!(r#"{{"kind": "single", {}, "answers": [{{"text": ["A", "A"], "right": true, "explain": {{"en": "Yes"}}}}, {{"text": ["B", "B"]}}]}}"#, stem), "Frage 1, Antwort 1: die Erklärung fehlt auf Deutsch.");
    bad(t, "an entry without English", format!(r#"{{"kind": "order", {}, "order": [["a", "a"], "b"]}}"#, stem), "Frage 1, Eintrag 2: der Text fehlt auf Englisch.");
    // no explanation at all is a note: the course can be offered
    let q = one(&format!(r#"{{"kind": "single", "stem": ["F", "Q"], {}}}"#, ab));
    t.eq("no explanation", (fatal(&q.issues()), notes(&q.issues())), (Vec::new(), alloc::vec![s("Frage 1 hat keine Erklärung.")]));
    // the whole file
    let q = Quiz::from_text(r#"{"format": "jfn-quiz", "version": 1}"#).unwrap_or_default();
    t.eq("no questions", fatal(&q.issues()), alloc::vec![s("Das Quiz hat keine Fragen.")]);
    let mut q = good();
    q.pass = 101;
    t.eq("over 100 %", fatal(&q.issues()), alloc::vec![s("Die Bestehensgrenze (pass) ist 101; höchstens 100 %.")]);
    let mut q = good();
    q.title = Text::new("Nur Deutsch", "");
    t.eq("a title without English", fatal(&q.issues()), alloc::vec![s("Der Titel des Quiz fehlt auf Englisch.")]);
    let mut q = good();
    let one_q = q.questions[0].clone();
    q.questions = alloc::vec![one_q; 65];
    t.check("too many questions", fatal(&q.issues()).contains(&s("Das Quiz hat mehr als 64 Fragen.")));
    let mut q = good();
    let a = q.questions[0].answers[0].clone();
    q.questions[0].answers = alloc::vec![a; 13];
    t.check("too many answers", fatal(&q.issues()).contains(&s("Frage 1 hat mehr als 12 Antworten.")));
    // with its step, as jfn-sim course says it
    t.eq("with the step", fatal(&quiz_issues("q", StepKind::Quiz, r#"{"format": "jfn-quiz", "version": 1}"#)), alloc::vec![s("Schritt q: Das Quiz hat keine Fragen.")]);
    t.eq("no JSON with the step", fatal(&quiz_issues("q", StepKind::Exam, "[1]")), alloc::vec![s("Schritt q: Die Datei enthält kein Quiz.")]);
    t.eq("a time limit in practice", notes(&quiz_issues("q", StepKind::Quiz, GOOD)), alloc::vec![s("Schritt q: Zeitlimit und Versuche gelten nur in einem Test (kind „exam“).")]);
    t.eq("in a test it counts", quiz_issues("q", StepKind::Exam, GOOD), Vec::<Problem>::new());
    // through jfn-sim course: quizzes, tests and fault hunts of a folder
    let root = "mem://jfn-selftest/quiz";
    let put = |rel: &str, text: &str| {
        plat::write_file(&format!("{}/{}", root, rel), text.as_bytes());
    };
    let course = |kind: &str, file: &str| format!(r#"{{"format": "jfn-course", "version": 1, "course_version": 1, "title": ["K", "K"], "summary": ["S", "S"], "units": [{{"title": ["U", "U"], "steps": [{{"id": "a", "kind": "{}", "title": ["A", "A"], "file": "{}"}}]}}]}}"#, kind, file);
    put("a/course.json", &course("quiz", "quiz/a.json"));
    put("a/quiz/a.json", r#"{"format": "jfn-quiz", "version": 1, "questions": [{"kind": "text", "stem": ["F", "Q"], "explain": ["E", "E"], "accept": ["(("]}]}"#);
    let (lines, errors) = jfn_run::check_course(&format!("{}/a", root));
    t.eq("a bad pattern in jfn-sim course", (lines, errors), (alloc::vec![s("Fehler: Schritt a: Frage 1: das Muster „((“ lässt sich nicht übersetzen: Unmatched ( or \\(\n")], 1));
    put("b/course.json", &course("exam", "quiz/fehlt.json"));
    let (lines, errors) = jfn_run::check_course(&format!("{}/b", root));
    t.eq("its file missing", (lines, errors), (alloc::vec![s("Fehler: Schritt a: die Datei quiz/fehlt.json fehlt.\n")], 1));
    put("c/course.json", &course("quiz", "quiz/a.txt"));
    let (lines, _) = jfn_run::check_course(&format!("{}/c", root));
    t.check("the wrong ending", lines.iter().any(|l| l.starts_with("Fehler: Schritt 1: die Datei quiz/a.txt endet nicht auf .json.")));
    put("d/course.json", &course("fault", "labs/a.jfnet"));
    put("d/labs/a.jfnet", &Project::new("Frei", 1).to_text());
    let (lines, errors) = jfn_run::check_course(&format!("{}/d", root));
    t.eq("a fault hunt without a task", (lines, errors), (alloc::vec![s("Fehler: Schritt a: die Datei hat keine Aufgabe.\n")], 1));
    put("e/course.json", &course("exam", ""));
    let (lines, errors) = jfn_run::check_course(&format!("{}/e", root));
    t.eq("announced, without a file", (lines, errors), (Vec::new(), 0));
}

// ---------------------------------------------------------------- grading

fn grade_t(t: &mut T) {
    let q = good();
    let [a, b, c, d, e] = [0, 1, 2, 3, 4].map(|k| q.questions[k].clone());
    let blank = |x: &Question| x.blank(&[]);
    let got = |m: Marked| (m.got, m.right);
    // single: the right one or nothing
    let with = |x: &Question, f: &dyn Fn(&mut Reply)| {
        let mut r = x.blank(&[]);
        f(&mut r);
        r
    };
    t.eq("single: right", got(a.grade(&with(&a, &|r| r.choice = 1))), (2000, true));
    t.eq("single: wrong", got(a.grade(&with(&a, &|r| r.choice = 0))), (0, false));
    t.eq("single: nothing", got(a.grade(&blank(&a))), (0, false));
    t.eq("single: out of range", got(a.grade(&with(&a, &|r| r.choice = 9))), (0, false));
    t.check("single: answered", a.answered(&with(&a, &|r| r.choice = 0)) && !a.answered(&blank(&a)));
    // multi: partial credit, never below nothing
    let marks = |v: [i32; 4]| with(&b, &|r| r.marks = v.to_vec());
    t.eq("multi: all right", got(b.grade(&marks([1, 1, 0, 0]))), (1000, true));
    t.eq("multi: one of two", got(b.grade(&marks([1, 0, 0, 0]))), (500, false));
    t.eq("multi: one right, one wrong", got(b.grade(&marks([1, 0, 1, 0]))), (0, false));
    t.eq("multi: both right, one wrong", got(b.grade(&marks([1, 1, 1, 0]))), (500, false));
    t.eq("multi: all marked", got(b.grade(&marks([1, 1, 1, 1]))), (0, false));
    t.eq("multi: wrong only", got(b.grade(&marks([0, 0, 1, 1]))), (0, false));
    t.eq("multi: nothing", got(b.grade(&marks([0, 0, 0, 0]))), (0, false));
    t.check("multi: answered", b.answered(&marks([0, 1, 0, 0])) && !b.answered(&marks([0, 0, 0, 0])));
    let mut three = b.clone();
    three.answers[2].right = true;
    three.points = Some(2);
    t.eq("multi: two of three, two points", three.grade(&with(&three, &|r| r.marks = alloc::vec![1, 1, 0, 0])).got, 2000 * 2 / 3);
    // text: as a whole, without the spaces around, capitals alike unless it says so
    let typed = |x: &Question, v: &str| {
        let mut r = x.blank(&[]);
        r.text = s(v);
        got(x.grade(&r)).1
    };
    for ok in ["10.0.0.0/24", "10.0.0.0 / 24", "  10.0.0.0/24  ", "10.0.0.0  /  24", "netz", "Netz Eins", "NETZ"] {
        t.check(&format!("text: {:?} counts", ok), typed(&c, ok));
    }
    for no in ["", "   ", "10.0.0.0/25", "10a0b0c0/24", "x10.0.0.0/24", "10.0.0.0/24x", "netz zwei", "(("] {
        t.check(&format!("text: {:?} does not", no), !typed(&c, no));
    }
    let mut case = c.clone();
    case.case = true;
    t.check("text: capitals count when it says so", typed(&case, "netz") && !typed(&case, "NETZ"));
    t.check("text: answered", c.answered(&with(&c, &|r| r.text = s("x"))) && !c.answered(&with(&c, &|r| r.text = s("  "))));
    // number: within its tolerance, with a comma or a point
    for ok in ["2.5", "2,5", " 2,50 ", "2", "3", "2.0", "3,000000"] {
        t.check(&format!("number: {:?} counts", ok), typed(&d, ok));
    }
    for no in ["", "1.99", "3.01", "-2,5", "zwei", "2.5.1", "1e1", "2,5 Hosts"] {
        t.check(&format!("number: {:?} does not", no), !typed(&d, no));
    }
    let mut exact = d.clone();
    exact.value = agent_core::jmap::Value::u64(254);
    exact.tolerance = agent_core::jmap::Value::Null;
    t.check("number: no tolerance", typed(&exact, "254") && !typed(&exact, "253") && !typed(&exact, "255"));
    t.eq("the value as text", exact.value_text(), s("254"));
    t.eq("fixed: whole", fixed("254"), Some(254_000_000));
    t.eq("fixed: a comma", fixed("0,5"), Some(500_000));
    t.eq("fixed: a point", fixed("-1.25"), Some(-1_250_000));
    t.eq("fixed: only decimals", fixed(".5"), Some(500_000));
    t.eq("fixed: too many decimals", fixed("1.1234567"), None);
    t.eq("fixed: too long", fixed("1234567890123456789"), None);
    for no in ["", "-", ".", "a", "1 2", "1,2,3", "+1"] {
        t.eq(&format!("fixed: {:?}", no), fixed(no), None);
    }
    // order: every entry in its place
    let order = |v: &[usize]| with(&e, &|r| r.order = v.to_vec());
    t.eq("order: right", got(e.grade(&order(&[0, 1, 2]))), (1000, true));
    t.eq("order: one swap", got(e.grade(&order(&[1, 0, 2]))), (0, false));
    t.eq("order: too short", got(e.grade(&order(&[0, 1]))), (0, false));
    t.check("order: always answered", e.answered(&blank(&e)));
    // the whole
    let mut r: Vec<Reply> = q.questions.iter().map(|x| x.blank(&[])).collect();
    t.eq("nothing given", q.grade(&r).percent(), 0);
    r[0].choice = 1;
    r[1].marks = alloc::vec![1, 0, 0, 0];
    let sc = q.grade(&r);
    t.eq("partly", (sc.got, sc.total, sc.percent()), (2500, 6000, 41));
    r[1].marks = alloc::vec![1, 1, 0, 0];
    r[2].text = s("netz");
    r[3].text = s("2,5");
    r[4].order = alloc::vec![0, 1, 2];
    let sc = q.grade(&r);
    t.eq("all", (sc.got, sc.percent(), sc.marks.iter().all(|m| m.right)), (6000, 100, true));
    r[4].order = alloc::vec![1, 0, 2];
    t.eq("rounded down: 100 only when all is right", q.grade(&r).percent(), 83);
}

fn plan_t(t: &mut T) {
    let q = good();
    let p = q.plan(7);
    t.eq("the same seed, the same order", q.plan(7), p.clone());
    let sorted = |v: &[usize]| {
        let mut x = v.to_vec();
        x.sort_unstable();
        x
    };
    t.eq("every question once", sorted(&p.questions), (0..5).collect::<Vec<_>>());
    t.eq("every answer once", sorted(&p.answers[1]), (0..4).collect::<Vec<_>>());
    t.eq("every entry once", sorted(&p.answers[4]), (0..3).collect::<Vec<_>>());
    t.check("another seed, another order", (1..20).any(|k| q.plan(k).questions != p.questions));
    t.check("answers change too", (1..20).any(|k| q.plan(k).answers[0] != p.answers[0]));
    // the sequence of the generator is part of the format of a script's expectations
    t.eq("a fixed result for a fixed seed", q.plan(0x4A46_4E41_4B32).questions.len(), 5);
    // without shuffle: as in the file, only the entries of an order question move, and never to the solution
    let mut plain = q.clone();
    plain.shuffle = false;
    for seed in 0..200 {
        let x = plain.plan(seed);
        t.eq("no shuffle: the questions", x.questions.as_slice(), &[0, 1, 2, 3, 4]);
        t.eq("no shuffle: the answers", x.answers[0].as_slice(), &[0, 1, 2]);
        if x.answers[4] == [0, 1, 2] {
            t.check(&format!("seed {} starts in the right order", seed), false);
        }
    }
    let mut shuffled = q.clone();
    shuffled.shuffle = true;
    for seed in 0..200 {
        if shuffled.plan(seed).answers[4] == [0, 1, 2] {
            t.check(&format!("seed {} starts solved", seed), false);
        }
    }
    // two entries can only start swapped
    let two = one(r#"{"kind": "order", "stem": ["F", "Q"], "explain": ["E", "E"], "order": [["a", "a"], ["b", "b"]]}"#);
    for seed in 0..50 {
        t.eq("two entries", two.plan(seed).answers[0].as_slice(), &[1, 0]);
    }
    t.check("a reply starts there", {
        let a = Attempt::new(&two, 3, 0);
        a.replies[0].order == [1, 0] && !two.questions[0].grade(&a.replies[0]).right
    });
}

fn attempt_t(t: &mut T) {
    let q = good();
    let mut a = Attempt::new(&q, 5, 1_000_000);
    t.eq("its seed and plan", (a.seed, a.plan.clone()), (5, q.plan(5)));
    t.eq("no end", a.end.clone(), None);
    t.eq("two minutes", a.limit_us, 120_000_000);
    t.eq("the time left", a.remaining(31_000_000), Some(90_000_000));
    t.check("not up yet", !a.time_up(120_999_999));
    t.check("up at the limit", a.time_up(121_000_000));
    t.eq("what is left at the end", a.remaining(500_000_000), Some(0));
    t.eq("all unanswered but the order", a.unanswered(&q), 4);
    a.replies[0].choice = 1;
    a.replies[1].marks[0] = 1;
    t.eq("two more answered", a.unanswered(&q), 2);
    // out of time: at the limit, whenever it was noticed
    let e = a.finish(&q, 700_000_000, true).clone();
    t.eq("out of time", (e.timeout, e.at_us, e.score.got), (true, 121_000_000, 2500));
    t.check("it failed", !e.passed);
    t.check("it ended", !a.time_up(900_000_000));
    t.eq("the clock stands at its end", a.remaining(900_000_000), Some(0));
    // graded once: what comes later changes nothing
    a.replies[1].marks[1] = 1;
    let again = a.finish(&q, 800_000_000, false).clone();
    t.eq("once", again, e);
    // handed in: at the time it was
    let mut b = Attempt::new(&q, 5, 0);
    b.replies[0].choice = 1;
    b.replies[1].marks = alloc::vec![1, 1, 0, 0];
    b.replies[2].text = s("netz");
    t.eq("handed in early", b.remaining(30_000_000), Some(90_000_000));
    let e = b.finish(&q, 30_000_000, false).clone();
    t.eq("handed in", (e.timeout, e.at_us, e.score.percent(), e.passed), (false, 30_000_000, 66, true));
    t.eq("the time stands", b.remaining(60_000_000), Some(90_000_000));
    // no limit
    let mut free = q.clone();
    free.minutes = 0;
    let c = Attempt::new(&free, 1, 0);
    t.eq("no limit", (c.remaining(1 << 40), c.time_up(1 << 40)), (None, false));
    // attempts
    t.eq("three of two", (q.attempts_left(0), q.attempts_left(1), q.attempts_left(2), q.attempts_left(3)), (Some(2), Some(1), Some(0), Some(0)));
    t.check("may start", q.can_start(0) && q.can_start(1) && !q.can_start(2) && !q.can_start(9));
    t.check("no limit of attempts", free.can_start(100) || free.attempts != 0);
    let mut any = q.clone();
    any.attempts = 0;
    t.eq("any number", (any.attempts_left(100), any.can_start(100)), (None, true));
    // what passes
    let mut full = q.clone();
    full.pass = 0;
    let mut d = Attempt::new(&full, 1, 0);
    d.replies[0].choice = 1;
    t.check("without pass all must be right", !d.finish(&full, 1, false).passed);
}

fn progress_t(t: &mut T) {
    let mut p = Progress::default();
    t.eq("no result", (p.best_of("k", "x"), p.tries_of("k", "x")), (None, 0));
    p.record("k", "x", 40, 100);
    t.eq("a result", (p.best_of("k", "x"), p.tries_of("k", "x")), (Some(40), 1));
    t.check("it failed: not done", !p.is_done("k", "x"));
    p.record("k", "x", 20, 200);
    t.eq("the best stays", (p.best_of("k", "x"), p.tries_of("k", "x")), (Some(40), 2));
    t.eq("the first time stays", p.courses["k"].steps["x"].when, 100);
    p.record("k", "x", 90, 300);
    t.eq("a better one", (p.best_of("k", "x"), p.tries_of("k", "x")), (Some(90), 3));
    t.check("done when it passes", p.mark_done("k", "x", 300));
    p.record("k", "x", 10, 400);
    t.eq("done stays, the best too", (p.is_done("k", "x"), p.best_of("k", "x")), (true, Some(90)));
    // 0 % is a result
    p.record("k", "null", 0, 500);
    t.eq("0 %", p.best_of("k", "null"), Some(0));
    // what practice reached: only more
    t.check("practice", p.note_best("k", "y", 50));
    t.check("less is nothing new", !p.note_best("k", "y", 40));
    t.check("the same neither", !p.note_best("k", "y", 50));
    t.check("more is", p.note_best("k", "y", 70));
    t.check("nothing is nothing", !p.note_best("k", "z", 0));
    t.eq("practice counts no attempts", (p.best_of("k", "y"), p.tries_of("k", "y")), (Some(70), 0));
    // through the file; an older file reads
    let q = Progress::from_text(&p.to_text());
    t.eq("round trip", (q.best_of("k", "x"), q.tries_of("k", "x"), q.best_of("k", "y"), q.is_done("k", "x")), (Some(90), 4, Some(70), true));
    let old = Progress::from_text(r#"{"format": "jfn-progress", "version": 1, "courses": {"k": {"steps": {"a": {"state": "done", "when": 5, "best": 0}}}}}"#);
    t.eq("an older file", (old.is_done("k", "a"), old.tries_of("k", "a"), old.best_of("k", "a")), (true, 0, None));
    // through the window's functions: a result, done when it passed; a reset takes the session's pages too
    let path = "mem://jfn-selftest/quiz/progress.json";
    t.check("written", store::write_progress(path, &p));
    t.eq("read", store::read_progress(path).tries_of("k", "x"), 4);
    t.check("reset", p.clear("k"));
    t.eq("all gone", (p.best_of("k", "x"), p.tries_of("k", "x")), (None, 0));
}

fn requires_t(t: &mut T) {
    let text = r#"{"format": "jfn-course", "version": 1, "course_version": 1, "title": ["K", "K"], "summary": ["S", "S"], "units": [{"title": ["U", "U"], "steps": [
        {"id": "a", "kind": "lesson", "title": ["A", "A"], "file": "a.md"},
        {"id": "b", "kind": "quiz", "title": ["B", "B"], "file": "quiz/b.json"},
        {"id": "x", "kind": "exam", "title": ["X", "X"], "file": "quiz/x.json", "requires": ["a", "b"]},
        {"id": "y", "kind": "lesson", "title": ["Y", "Y"], "file": "y.md"},
        {"id": "z", "kind": "fault", "title": ["Z", "Z"], "file": ""}]}]}"#;
    let c = Course::from_text(text).unwrap_or_default();
    t.eq("no issues", c.issues(), Vec::<Problem>::new());
    t.eq("a default of none", c.step_at(0).map(|s| s.requires.len()), Some(0));
    t.eq("what can be opened", c.ready_count(), 4);
    t.check("announced", c.step_at(4).is_some_and(|s| !s.ready()));
    t.check("a quiz with a file can be", c.step_at(1).is_some_and(Step::ready));
    use StepState::*;
    let mut p = Progress::default();
    t.eq("locked at the start", p.states(&c, "k"), alloc::vec![Current, Open, Locked, Open, NotYet]);
    t.eq("next: the first", p.next_open(&c, "k"), Some(0));
    p.mark_done("k", "a", 1);
    t.eq("one of two", p.states(&c, "k"), alloc::vec![Done, Current, Locked, Open, NotYet]);
    p.mark_done("k", "b", 2);
    t.eq("open now", p.states(&c, "k"), alloc::vec![Done, Done, Current, Open, NotYet]);
    t.eq("next: the exam", p.next_open(&c, "k"), Some(2));
    // a locked step is passed over; next_open goes on to what can be done
    let mut q = Progress::default();
    q.mark_done("k", "y", 1);
    q.mark_done("k", "a", 1);
    t.eq("with the lesson after it done", q.states(&c, "k"), alloc::vec![Done, Current, Locked, Done, NotYet]);
    let mut r = Progress::default();
    r.mark_done("k", "a", 1);
    r.mark_done("k", "y", 1);
    r.record("k", "b", 10, 1);
    t.eq("a failed try does not open it", (r.next_open(&c, "k"), r.states(&c, "k")[2]), (Some(1), Locked));
    let bad = |req: &str| {
        let x = text.replace("\"requires\": [\"a\", \"b\"]", req);
        fatal(&Course::from_text(&x).unwrap_or_default().issues())
    };
    t.eq("a later step", bad(r#""requires": ["y"]"#), alloc::vec![s("Schritt x: der verlangte Schritt y steht nicht davor.")]);
    t.eq("itself", bad(r#""requires": ["x"]"#), alloc::vec![s("Schritt x: der verlangte Schritt x steht nicht davor.")]);
    t.eq("no such step", bad(r#""requires": ["weg"]"#), alloc::vec![s("Schritt x: der verlangte Schritt weg steht nicht davor.")]);
}

// ---------------------------------------------------------------- labs: start faults and locks

// PC1 and PC2 with addresses, on a cable each to a switch; R1 beyond it
fn net() -> Project {
    let mut p = Project::new("Netz", 1);
    let pc = |p: &mut Project, name: &str, addr: &str| {
        let id = p.add_device("pc", 0, 0).unwrap_or(0);
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = format!("hostname {}\nip addr add {}/24 dev eth0\n", name, addr);
        }
        id
    };
    let a = pc(&mut p, "PC1", "10.0.0.1");
    let b = pc(&mut p, "PC2", "10.0.0.2");
    let sw = p.add_device("jf-s24p", 0, 0).unwrap_or(0);
    if let Some(d) = p.device_mut(sw) {
        d.name = s("SW1");
    }
    let end = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let _ = p.connect(&end(a, "eth0"), &end(sw, "GigabitEthernet1/0/1"), None);
    let _ = p.connect(&end(b, "eth0"), &end(sw, "GigabitEthernet1/0/2"), None);
    p
}

fn with_faults(mut p: Project, faults: &[&str]) -> Project {
    p.lab = Some(Lab { title: Text::new("L", "L"), start_faults: faults.iter().map(|f| s(f)).collect(), assess: Some(Assess { feedback: Feedback::Score, time_limit_s: 0, items: alloc::vec![Item { what: s("check"), checks: alloc::vec![s("PC1 reaches 10.0.0.2")], points: 1, ..Item::default() }], target: Target { devices: p.devices.clone(), links: p.links.clone() } }), ..Lab::default() });
    p
}

fn lab_t(t: &mut T) {
    // the defaults: nothing hidden, nothing in the file, every field as before
    t.eq("no lock", Lock::default(), Lock { problems: false, faults: false, answer: false });
    let mut p = Project::new("Lab", 1);
    p.lab = Some(Lab { title: Text::new("T", "T"), ..Lab::default() });
    let text = p.to_text();
    for word in ["start_faults", "\"lock\"", "\"pass\""] {
        t.check(&format!("{} stays out of the file", word), !text.contains(word));
    }
    let back = Project::from_text(&text).ok().and_then(|q| q.lab);
    t.eq("an old lab reads as ever", back.as_ref().map(|l| (l.start_faults.len(), l.lock.clone(), l.pass)), Some((0, Lock::default(), 0)));
    t.eq("it passes at 100 %", back.map(|l| l.pass_percent()), Some(100));
    // the app's labs have none of it and write the same
    for name in crate::workspace::task::labs().iter().map(|l| l.path.clone()).chain(crate::workspace::task::assessments().iter().map(|l| l.path.clone())) {
        let Ok(q) = crate::doc::file::read(&name) else {
            t.check(&format!("{} loads", name), false);
            continue;
        };
        let lab = q.lab.clone().unwrap_or_default();
        t.eq(&format!("{}: no start faults, no lock", name), (lab.start_faults.len(), lab.lock), (0, Lock::default()));
        t.check(&format!("{}: nothing new in the file", name), !q.to_text().contains("start_faults"));
    }
    // with them: through the file
    let mut q = with_faults(net(), &["link PC1 eth0", "power PC2 for 5s"]);
    if let Some(l) = q.lab.as_mut() {
        l.lock = Lock { problems: true, faults: true, answer: false };
        l.pass = 80;
    }
    let text = q.to_text();
    t.check("in the file", text.contains("\"start_faults\": [") && text.contains("\"lock\": {") && text.contains("\"pass\": 80"));
    let back = Project::from_text(&text).ok().and_then(|x| x.lab).unwrap_or_default();
    t.eq("round trip", (back.start_faults, back.lock, back.pass), (alloc::vec![s("link PC1 eth0"), s("power PC2 for 5s")], Lock { problems: true, faults: true, answer: false }, 80));
    // the scenario's syntax
    t.eq("a fault alone", scenario::parse_fault_text("link R1 Gi0/1"), Ok((FaultSpec::Link { dev: s("R1"), port: s("Gi0/1") }, None)));
    t.eq("with its word and a time", scenario::parse_fault_text("fault power SRV for 5s"), Ok((FaultSpec::Power { dev: s("SRV") }, Some(5 * SEC))));
    t.check("no fault", scenario::parse_fault_text("").is_err() && scenario::parse_fault_text("fault").is_err() && scenario::parse_fault_text("kaputt R1").is_err());
    t.eq("the devices it names", FaultSpec::Partition { a: alloc::vec![s("A")], b: alloc::vec![s("B")] }.devices(), alloc::vec!["A", "B"]);
    t.eq("the cable it means", FaultSpec::Link { dev: s("R1"), port: s("Gi0") }.cable(), Some(("R1", "Gi0")));
    // what is wrong with them
    let issues = |faults: &[&str]| {
        let x = with_faults(net(), faults);
        x.lab.as_ref().map_or(Vec::new(), |l| l.issues(&x).iter().map(|m| m.text()).collect::<Vec<_>>())
    };
    t.eq("all right", issues(&["link PC1 eth0", "power SW1", "stop PC2 web"]), Vec::<String>::new());
    t.check("syntax", issues(&["kaputt"]).iter().any(|m| m.starts_with("Startfehler 1: „kaputt“ ist kein Fehler:")));
    t.eq("no such device", issues(&["power R9"]), alloc::vec![s("Startfehler 1: das Gerät R9 gibt es im Projekt nicht.")]);
    t.eq("no cable there", issues(&["link SW1 Gi1/0/9"]), alloc::vec![s("Startfehler 1: an SW1 Gi1/0/9 steckt kein Kabel.")]);
    let mut over = with_faults(net(), &[]);
    if let Some(l) = over.lab.as_mut() {
        l.pass = 101;
    }
    t.eq("pass over 100", over.lab.as_ref().map(|l| l.issues(&over).iter().map(|m| m.text()).collect::<Vec<_>>()), Some(alloc::vec![s("Die Bestehensgrenze (pass) ist 101; höchstens 100 %.")]));
    // a fault hunt's file
    t.check("not a project", fault_issues("s", "{").iter().all(|p| p.fatal));
    t.eq("no task", fatal(&fault_issues("s", &Project::new("Frei", 1).to_text())), alloc::vec![s("Schritt s: die Datei hat keine Aufgabe.")]);
    let mut plain = net();
    plain.lab = Some(Lab { title: Text::new("T", "T"), goals: alloc::vec![jfn_model::Goal { text: Text::new("Z", "G"), checks: alloc::vec![s("PC1 reaches 10.0.0.2")], hints: Vec::new() }], ..Lab::default() });
    t.eq("goals are not enough", fatal(&fault_issues("s", &plain.to_text())), alloc::vec![s("Schritt s: eine Fehlersuche wird bewertet; die Datei hat keine Kriterien (assess).")]);
    t.eq("graded: fine", fault_issues("s", &with_faults(net(), &["link PC1 eth0"]).to_text()), Vec::<Problem>::new());
}

fn start_t(t: &mut T) {
    let p = with_faults(net(), &["link PC1 eth0", "power PC2"]);
    let pc1_link = p.links[0].id;
    // what the engine gets, in the order of the file
    let e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(1));
    let (acts, bad) = jfn_sim::runner::start_actions(&p, &e);
    t.eq("no refusal", bad, Vec::<String>::new());
    let want = alloc::vec![Action::Fault { fault: Fault::Unplug { link: pc1_link }, dur: None }, Action::Fault { fault: Fault::Power { dev: p.devices[1].id }, dur: None }];
    t.eq("two actions", format!("{:?}", acts), format!("{:?}", want));
    // a project without a lab or start faults has none
    let (acts, bad) = jfn_sim::runner::start_actions(&net(), &e);
    t.eq("no lab, no actions", (acts.len(), bad.len()), (0, 0));
    let (acts, bad) = jfn_sim::runner::start_actions(&with_faults(net(), &["link R9 eth0", "stop"]), &e);
    t.eq("what cannot be resolved", (acts.len(), bad.len()), (0, 2));
    // the window's simulator: at once, before the clock moves
    let mut sim = Sim::new(&p);
    let active = |sim: &Sim| sim.engine().active_faults().map(|m| m.fault.clone()).collect::<Vec<_>>();
    t.eq("two faults at time 0", (active(&sim).len(), sim.engine().now()), (2, 0));
    t.check("PC2 is off", !sim.view.devices.iter().find(|d| d.dev == p.devices[1].id).is_some_and(|d| d.power));
    let _ = sim.send(Cmd::RunFor(2 * SEC));
    t.eq("still there", active(&sim).len(), 2);
    // a project without them is as it was
    let plain = net();
    let mut old = Sim::new(&plain);
    let _ = old.send(Cmd::RunFor(2 * SEC));
    t.eq("none without them", old.engine().faults().len(), 0);
    // a fault with a time heals by itself
    let mut brief = Sim::new(&with_faults(net(), &["link PC1 eth0 for 1s"]));
    let _ = brief.send(Cmd::RunFor(2 * SEC));
    t.eq("for a while", (brief.engine().faults().len(), brief.engine().active_faults().count()), (1, 0));
    // deterministic: the same run twice, byte for byte
    let run = |p: &Project| {
        let mut x = Sim::new(p);
        let _ = x.send(Cmd::Cli { dev: p.devices[0].id, line: s("ping -c 2 10.0.0.2") });
        let _ = x.send(Cmd::RunFor(5 * SEC));
        digest(x.engine())
    };
    let q = with_faults(net(), &["link PC2 eth0"]);
    t.eq("deterministic", run(&q), run(&q));
    t.check("the fault makes a difference", run(&q) != run(&net()));
    // K1: a replay from the recording ends in the same state, with the start faults as inputs
    let mut d = Inline::new(Engine::from_project(&q).unwrap_or_else(|_| Engine::new(1)));
    d.rewind_on(Budget::default());
    for a in jfn_sim::runner::start_actions(&q, &d.eng).0 {
        let _ = d.send(Cmd::Apply(a), 0);
    }
    t.eq("they are inputs", d.pos().inputs, 1);
    let _ = d.send(Cmd::Cli { dev: q.devices[0].id, line: s("ping -c 2 10.0.0.2") }, 0);
    let _ = d.send(Cmd::RunFor(3 * SEC), 0);
    let at = d.pos();
    let want = digest(&d.eng);
    let got = d.restore_pos(at, None).and_then(|mut r| {
        let rw = d.rewind()?;
        r.run(rw, usize::MAX).then(|| digest(&r.finish().eng))
    });
    t.eq("a replay", got, Some(want));
    // the window's simulator is a recording as well: the fault in the past
    let mut w = Sim::new(&q);
    let _ = w.send(Cmd::RunFor(3 * SEC));
    t.check("in the recording", w.rewind().is_some_and(|r| !r.inputs().1.is_empty()));
    // Simulation zurücksetzen: the faults come back, but not onto a cable the learner put in since
    let mut edited = q.clone();
    let old_link = edited.links[1].clone();
    edited.links[1] = Link { id: edited.next_id, ..old_link.clone() };
    edited.next_id += 1;
    let born: Vec<u32> = q.links.iter().map(|l| l.id).collect();
    t.eq("reset: back on the old cable", Sim::again(&q, &born).engine().active_faults().count(), 1);
    t.eq("reset: not on a new one", Sim::again(&edited, &born).engine().active_faults().count(), 0);
    // the same fault in a scenario's report
    let sc = scenario::parse("run 1s\nexpect PC1 not reaches 10.0.0.2\n").unwrap_or_default();
    let out = jfn_sim::run_scenario(&q, &sc, "start", false);
    t.check("the report names it", out.report.contains("Startfehler 1: Kabel PC2 eth0"));
    t.check("the scenario sees it", out.ok);
    let out = jfn_sim::run_scenario(&net(), &sc, "plain", false);
    t.check("without it, nothing changes", !out.report.contains("Startfehler") && !out.ok);
    // jfn-sim check: a fault the engine refuses
    let (lines, errors) = jfn_run::check_project(&with_faults(net(), &["stop PC1 web"]));
    t.eq("a refusal in jfn-sim check", (errors, lines.len()), (1, 1));
    t.check("what it says", lines.iter().any(|l| l.starts_with("Fehler: Startfehler: Den Dienst")));
    t.eq("a good one says nothing", jfn_run::check_project(&q).1, 0);
    // the learner's way out with the faults locked: a power fault by the switch, a pulled cable by a new one
    let f = with_faults(net(), &["power PC2", "link PC1 eth0"]);
    let mut doc = crate::doc::Doc::new();
    doc.replace(f.clone(), None);
    doc.sim.start();
    let off = |doc: &crate::doc::Doc| !doc.sim.view.devices.iter().find(|d| d.dev == f.devices[1].id).is_some_and(|d| d.power);
    t.check("off by the fault", off(&doc));
    if let Some(d) = doc.proj.device_mut(f.devices[1].id) {
        d.off = true;
    }
    doc.sim.sync(&doc.proj);
    if let Some(d) = doc.proj.device_mut(f.devices[1].id) {
        d.off = false;
    }
    doc.sim.sync(&doc.proj);
    t.check("on by the switch", !off(&doc));
    let l = doc.proj.links[0].clone();
    doc.proj.links.remove(0);
    doc.sim.sync(&doc.proj);
    let id = doc.proj.next_id;
    doc.proj.next_id += 1;
    doc.proj.links.push(Link { id, ..l });
    doc.sim.sync(&doc.proj);
    let _ = doc.sim.send(Cmd::RunFor(3 * SEC));
    let v = judge(&doc.proj, doc.sim.engine(), &scenario::parse_expectation("PC1 reaches 10.0.0.2").unwrap_or_else(|_| unreachable!()), &|_| String::new());
    t.check("a new cable carries", v.held);
}

fn diagnosis_t(t: &mut T) {
    let p = net();
    let (pc1, sw) = (p.devices[0].id, p.devices[2].id);
    let (dev_item, cable_item) = (
        Item { what: s("diagnosis"), dev: s("SW1"), id: sw, points: 2, explain: Text::new("Weil", "Because"), ..Item::default() },
        Item { what: s("diagnosis"), dev: s("PC1"), id: pc1, port: s("eth0"), peer: s("SW1"), peer_id: sw, peer_port: s("Gi1/0/1"), points: 3, ..Item::default() },
    );
    t.eq("its kind", dev_item.kind(), Some(jfn_model::lab::ItemKind::Diagnosis));
    t.check("the question gives nothing away", !item_text(&cable_item).text().contains("SW1") && !item_text(&dev_item).text().contains("SW1"));
    // its file
    let text = jfn_model::lab::ItemKind::Diagnosis.as_str();
    t.eq("in the file", text, "diagnosis");
    let mut q = with_faults(p.clone(), &[]);
    if let Some(a) = q.lab.as_mut().and_then(|l| l.assess.as_mut()) {
        a.items = alloc::vec![dev_item.clone(), cable_item.clone()];
    }
    let back = Project::from_text(&q.to_text()).ok().and_then(|x| x.lab).and_then(|l| l.assess).unwrap_or_default();
    t.eq("round trip", back.items, alloc::vec![dev_item.clone(), cable_item.clone()]);
    t.eq("no issues", q.lab.as_ref().map(|l| l.issues(&q).iter().map(|m| m.text()).collect::<Vec<_>>()), Some(Vec::new()));
    let wrong = |it: Item| {
        let mut x = with_faults(p.clone(), &[]);
        if let Some(a) = x.lab.as_mut().and_then(|l| l.assess.as_mut()) {
            a.items = alloc::vec![it];
        }
        x.lab.as_ref().map_or(Vec::new(), |l| l.issues(&x).iter().map(|m| m.text()).collect::<Vec<_>>())
    };
    t.eq("no device", wrong(Item { what: s("diagnosis"), points: 1, ..Item::default() }), alloc::vec![s("Kriterium 1 nennt kein Gerät.")]);
    t.eq("a device that is none", wrong(Item { what: s("diagnosis"), dev: s("R9"), points: 1, ..Item::default() }), alloc::vec![s("Kriterium 1: das Gerät R9 gibt es im Projekt nicht.")]);
    t.eq("a far end that is none", wrong(Item { what: s("diagnosis"), dev: s("SW1"), peer: s("R9"), points: 1, ..Item::default() }), alloc::vec![s("Kriterium 1: das Gerät R9 gibt es im Projekt nicht.")]);
    // what counts
    let e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(1));
    let end = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let right_cable = Pick::Cable(end(pc1, "eth0"), end(sw, "GigabitEthernet1/0/1"));
    t.check("the device", pick_right(&dev_item, &p, &e, &Pick::Dev(sw)));
    t.check("not another", !pick_right(&dev_item, &p, &e, &Pick::Dev(pc1)));
    t.check("not a cable", !pick_right(&dev_item, &p, &e, &right_cable));
    t.check("the cable", pick_right(&cable_item, &p, &e, &right_cable));
    t.check("from its other end", pick_right(&cable_item, &p, &e, &Pick::Cable(end(sw, "Gi1/0/1"), end(pc1, "eth0"))));
    t.check("not another cable", !pick_right(&cable_item, &p, &e, &Pick::Cable(end(p.devices[1].id, "eth0"), end(sw, "GigabitEthernet1/0/2"))));
    t.check("not a device", !pick_right(&cable_item, &p, &e, &Pick::Dev(sw)));
    let any = Item { port: String::new(), peer_port: String::new(), ..cable_item.clone() };
    t.check("any port between the two", pick_right(&any, &p, &e, &Pick::Cable(end(pc1, "eth0"), end(sw, "GigabitEthernet1/0/7"))));
    t.eq("the names", (pick_text(&p, &Pick::Dev(sw)), pick_text(&p, &right_cable)), (s("SW1"), s("PC1 eth0 – SW1 Gi1/0/1")));
    // graded when it is handed in, not before: the score cannot be asked
    let a = Assess { feedback: Feedback::Items, items: alloc::vec![dev_item.clone(), cable_item.clone()], target: Target { devices: p.devices.clone(), links: p.links.clone() }, ..Assess::default() };
    let g = Grader::new(&a);
    let out = |_: u32| String::new();
    let plain = g.grade(&p, &e, &out);
    t.eq("no picks", (plain.points, plain.total, plain.items[0].open), (0, 5, true));
    let picks = alloc::vec![Some(Pick::Dev(sw)), Some(Pick::Dev(pc1))];
    let open = g.grade_picks(&p, &e, &out, &picks, false);
    t.eq("before handing in: nothing counts", (open.points, open.held(), open.items[0].open, open.items[0].found.text()), (0, 0, true, String::new()));
    let done = g.grade_picks(&p, &e, &out, &picks, true);
    t.eq("handed in", (done.points, done.percent(), done.items[0].held, done.items[1].held, done.items[0].open), (2, 40, true, false, false));
    t.eq("what the wrong one said", done.items[1].found.text(), s("Ihre Antwort: PC1"));
    let none = g.grade_picks(&p, &e, &out, &[None, None], true);
    t.eq("no answer", (none.points, none.items[0].found.text()), (0, s("keine Antwort")));
    // the panel's run
    let mut run = Run::new(&a, 0);
    t.check("it is handed in without a limit", run.hand_in);
    run.picks[0] = Some(Pick::Dev(sw));
    run.grade(&p, &e, &out, 1, 1, 0);
    t.eq("while it runs", run.score.as_ref().map(|x| x.points), Some(0));
    run.finish(&p, &e, &out, 1, 1, 10, false);
    t.eq("at the end", run.score.as_ref().map(|x| (x.points, x.percent())), Some((2, 40)));
    t.check("no longer waiting", run.picking.is_none());
    let plain_assess = Assess { items: alloc::vec![Item { what: s("hostname"), dev: s("SW1"), value: agent_core::jmap::Value::string("SW1"), points: 1, ..Item::default() }], ..Assess::default() };
    t.check("other assessments as ever", !Run::new(&plain_assess, 0).hand_in);
    // the start network leaves it alone
    let tp = Project { lab: None, ..p.clone() };
    t.eq("the start network", jfn_sim::assess::start_network(&tp, &e, &[dev_item]).devices.len(), tp.devices.len());
}

fn negative_t(t: &mut T) {
    // the scenario language has it: "not reaches"
    t.eq("it parses", scenario::parse_expectation("PC1 not reaches 10.0.0.9"), Ok(jfn_model::scenario::Expect::Reach { dev: s("PC1"), host: s("10.0.0.9"), negate: true }));
    let p = net();
    let mut sim = Sim::new(&p);
    let _ = sim.send(Cmd::RunFor(2 * SEC));
    let check = |c: &str, sim: &Sim| judge(&p, sim.engine(), &scenario::parse_expectation(c).unwrap_or_else(|_| unreachable!()), &|_| String::new()).held;
    t.check("what answers does not hold", !check("PC1 not reaches 10.0.0.2", &sim));
    t.check("what does not answer holds", check("PC1 not reaches 10.0.0.9", &sim));
    t.check("the same as reaches", check("PC1 reaches 10.0.0.2", &sim) && !check("PC1 reaches 10.0.0.9", &sim));
    // an item with it: graded as it is now, so it can break again
    let a = Assess { items: alloc::vec![Item { what: s("check"), checks: alloc::vec![s("PC1 not reaches 10.0.0.2")], points: 1, ..Item::default() }], ..Assess::default() };
    let g = Grader::new(&a);
    let held = |sim: &Sim| g.grade(&p, sim.engine(), &|_| String::new()).points;
    t.eq("while PC2 answers", held(&sim), 0);
    let _ = sim.send(Cmd::Apply(Action::Fault { fault: Fault::Power { dev: p.devices[1].id }, dur: None }));
    t.eq("while PC2 is off", held(&sim), 1);
    let _ = sim.send(Cmd::Apply(Action::HealAll));
    let _ = sim.send(Cmd::RunFor(2 * SEC));
    t.eq("again: not sticky", held(&sim), 0);
    t.eq("its text", item_text(&Item { what: s("check"), checks: alloc::vec![s("PC1 not reaches 10.0.0.2")], ..Item::default() }).text(), s("PC1 erreicht 10.0.0.2 nicht"));
}

// ---------------------------------------------------------------- the sample course

fn sample_t(t: &mut T) {
    let Some(root) = plat::arg_value("--academy-dir") else { return };
    let dir = format!("{}/beispiel", root);
    let e = store::entry(&root, "beispiel");
    let Some(c) = e.usable() else {
        t.check("the sample is offered", false);
        return;
    };
    t.eq("every kind in it", StepKind::ALL.iter().filter(|k| c.steps().any(|s| s.kind() == Some(**k))).count(), 5);
    for step in c.steps().filter(|s| matches!(s.kind(), Some(StepKind::Quiz | StepKind::Exam))) {
        let name = format!("sample {}", step.id);
        let Some(text) = store::read_quiz(&dir, &step.file) else {
            t.check(&format!("{} reads", name), false);
            continue;
        };
        t.eq(&format!("{}: nothing to say", name), quiz_issues(&step.id, step.kind().unwrap_or_default(), &text), Vec::<Problem>::new());
        let low = text.to_lowercase();
        for word in ["networking academy", "netacad", "cisco", "packet tracer"] {
            t.check(&format!("{}: no {}", name, word), !low.contains(word));
        }
        let q = Quiz::from_text(&text).unwrap_or_default();
        t.check(&format!("{}: every kind", name), [QKind::Single, QKind::Multi, QKind::Text, QKind::Number, QKind::Order].iter().all(|k| q.questions.iter().any(|x| x.kind_of() == Some(*k))));
        // the right replies reach all the points; nothing given reaches none
        let right: Vec<Reply> = q.questions.iter().map(|x| right_reply(x)).collect();
        t.eq(&format!("{}: all right", name), q.grade(&right).percent(), 100);
        let plan = q.plan(1);
        let blank: Vec<Reply> = q.questions.iter().enumerate().map(|(i, x)| x.blank(&plan.answers[i])).collect();
        t.check(&format!("{}: nothing given", name), q.grade(&blank).percent() < q.pass_percent());
        t.eq(&format!("{}: the same order in a headless run", name), q.plan(1), plan);
    }
    t.eq("the test needs the quiz", c.steps().find(|s| s.id == "pruefung").map(|s| s.requires.clone()), Some(alloc::vec![s("wissen")]));
    // the fault hunt: without its fix not full, with it full
    let Some(step) = c.steps().find(|s| s.kind() == Some(StepKind::Fault)) else {
        t.check("a fault hunt", false);
        return;
    };
    let Ok(p) = crate::doc::file::read(&format!("{}/{}", dir, step.file)) else {
        t.check("the fault hunt loads", false);
        return;
    };
    let Some(lab) = p.lab.clone() else { return };
    let Some(a) = lab.assess.clone() else {
        t.check("it grades", false);
        return;
    };
    t.check("two languages", !lab.title.en.is_empty() && lab.intro.iter().all(|x| !x.en.is_empty()) && lab.hints.iter().all(|x| !x.en.is_empty()) && a.items.iter().all(|i| !i.text.en.is_empty()));
    t.eq("what it locks", lab.lock.clone(), Lock { problems: true, faults: true, answer: true });
    t.eq("what it starts with", lab.start_faults.clone(), alloc::vec![s("link SRV1 eth0")]);
    t.eq("what passes", lab.pass_percent(), 80);
    t.eq("the file has nothing to say", fault_issues(&step.id, &p.to_text()), Vec::<Problem>::new());
    t.eq("jfn-sim check", jfn_run::check_project(&p), (Vec::new(), 0));
    let grade = |project: &Project, picks: &[Option<Pick>]| {
        let mut sim = Sim::new(project);
        let _ = sim.send(Cmd::RunFor(5 * SEC));
        Grader::new(&a).grade_picks(project, sim.engine(), &|_| String::new(), picks, true)
    };
    let picks = |x: Option<Pick>| -> Vec<Option<Pick>> { a.items.iter().map(|i| if i.what == "diagnosis" { x.clone() } else { None }).collect() };
    let (r1, srv) = (p.find("R1").map_or(0, |d| d.id), p.find("SRV1").map_or(0, |d| d.id));
    let cable = Pick::Cable(PortRef { dev: r1, port: s("GigabitEthernet0/0/1") }, PortRef { dev: srv, port: s("eth0") });
    let broken = grade(&p, &picks(None));
    t.eq("broken: only that PC2 stays out", (broken.points, broken.total, broken.percent()), (2, 10, 20));
    let named = grade(&p, &picks(Some(cable.clone())));
    t.eq("broken, the cable named: not enough", named.percent() >= lab.pass_percent(), false);
    // the fix: the gateway of PC1, a new cable in place of the pulled one
    let mut fixed = p.clone();
    if let Some(d) = fixed.find("PC1").map(|d| d.id).and_then(|id| fixed.device_mut(id)) {
        d.startup_config = d.startup_config.replace("10.0.1.254", "10.0.1.1");
    }
    let old = fixed.link_at(srv, "eth0").map_or(0, |l| l.id);
    if let Some(l) = fixed.links.iter_mut().find(|l| l.id == old) {
        l.id = fixed.next_id;
    }
    fixed.next_id += 1;
    // (a reset of this project; a new load would pull the new cable again)
    let born: Vec<u32> = p.links.iter().map(|l| l.id).collect();
    let solved = |project: &Project, picks: &[Option<Pick>]| {
        let mut sim = Sim::again(project, &born);
        let _ = sim.send(Cmd::RunFor(5 * SEC));
        Grader::new(&a).grade_picks(project, sim.engine(), &|_| String::new(), picks, true)
    };
    t.eq("repaired, the cable named: full", solved(&fixed, &picks(Some(cable.clone()))).percent(), 100);
    t.eq("repaired, the wrong device named: not full", solved(&fixed, &picks(Some(Pick::Dev(r1)))).percent(), 80);
    t.eq("repaired, nothing named", solved(&fixed, &picks(None)).percent(), 80);
    // PC2 stays out: taking the access list away loses the points again
    if let Some(d) = fixed.find("R1").map(|d| d.id).and_then(|id| fixed.device_mut(id)) {
        d.startup_config = d.startup_config.replace(" ip access-group 101 in\n", "");
    }
    t.eq("the access list gone: not full", solved(&fixed, &picks(Some(cable))).percent(), 80);
}

// the reply that gets all the points of a question
fn right_reply(x: &Question) -> Reply {
    let mut r = x.blank(&[]);
    match x.kind_of() {
        Some(QKind::Single) => r.choice = x.answers.iter().position(|a| a.right).map_or(-1, |i| i as i32),
        Some(QKind::Multi) => r.marks = x.answers.iter().map(|a| a.right as i32).collect(),
        Some(QKind::Number) => r.text = x.value_text(),
        Some(QKind::Order) => r.order = (0..x.order.len()).collect(),
        Some(QKind::Text) => r.text = match x.accept.first().map(String::as_str) {
            Some(p) if p.contains("255") => s("255.255.255.0"),
            _ => s("10.1.2.0/24"),
        },
        None => {}
    }
    r
}
