// quiz -- the quizzes and tests of the Akademie as files: what a question is, what is wrong with one, how a
// reply is graded, an attempt with its order and its time.  Text in, records and messages out, no I/O and
// no clock (the caller says what its clock shows), so that the app, jfn-sim and the selftest grade alike.
//
//   academy/<kurs>/quiz/*.json   Quiz: format "jfn-quiz", version, questions, all texts in German and
//                                English together (lab::Text: a pair ["Deutsch", "English"])
//
//   Quiz::from_text(text) -> Result<Quiz, Msg>   the wrong format or a newer version is refused
//   quiz.issues() -> Vec<Problem>            what is wrong in the file: no questions, a question without a
//                                            right answer, a pattern that does not compile, a number that
//                                            is none, a language missing, each with its question
//   quiz.total() / pass_percent() / attempts_left(used) / can_start(used)
//   quiz.plan(seed) -> Plan                  the order of an attempt: the questions, per question its
//                                            answers or the entries to put in order
//   question.kind_of() / worth() / blank(&shown) / answered(&reply) / grade(&reply) -> Marked
//   Attempt::new(&quiz, seed, clock)         an attempt from `clock` on (µs of the app's clock): its plan,
//                                            its replies, its time limit
//   attempt.remaining(clock) / time_up(clock) / unanswered(&quiz) / finish(&quiz, clock, timeout)
//   Marked { got, max, right }               thousandths of a point, so that partial credit stays exact
//   Score { got, total, marks } / percent()  rounded down: 100 only when everything is right
//   fixed(text) -> Option<i128>              a number in millionths; a decimal comma counts as a point
//
//   {
//     "format": "jfn-quiz", "version": 1,
//     "title": ["Deutsch", "English"],       optional: the step's title is what the page shows
//     "pass": 70,                            per cent to pass (default 100)
//     "minutes": 10, "attempts": 3,          a test's time limit and attempts (default: none)
//     "shuffle": true,                       questions and answers in another order per attempt
//     "questions": [
//       {"kind": "single", "stem": [...], "points": 2, "explain": [...],
//        "answers": [{"text": [...], "right": true, "explain": [...]}, {"text": [...]}]},
//       {"kind": "multi", ... the same with several right answers},
//       {"kind": "text", "stem": [...], "accept": ["10\\.0\\.0\\.0 */ *24"], "case": false},
//       {"kind": "number", "stem": [...], "value": 254, "tolerance": 0},
//       {"kind": "order", "stem": [...], "order": [[...], [...], [...]]}]
//   }
//
// A stem and a question's explanation are Markdown; the rest is plain text.  "points" defaults to 1.
//
// Grading: single: the right answer or nothing.  multi: partial credit: (right ones chosen - wrong ones
// chosen) / right ones, never below 0; "right" is only all of them and none else.  text: the reply without
// spaces around it and with runs of spaces as one must match one pattern of "accept" as a whole (POSIX
// extended, base::pattern; "case": true tells capitals apart).  number: within "tolerance" of "value"; a
// decimal comma or point reads the same.  order: every entry in its place; the entries are always shown in
// another order, also without "shuffle", or the question would answer itself.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::{Json, Value};
use agent_core::{record, wire_enum};
use base::pattern::{Regex, Syntax};

use crate::academy::{fatal, note, parse, version_ok, Problem};
use crate::lab::Text;
use crate::msg;
use crate::msg::{phrase, Msg};

/// `format` of a quiz's file
pub const QUIZ_FORMAT: &str = "jfn-quiz";
/// the most questions of a quiz and answers of a question (the page's block of ids has room for them)
pub const QUESTIONS_MAX: usize = 64;
pub const ANSWERS_MAX: usize = 12;
/// what a point is worth in a Marked
pub const MILLI: u32 = 1000;

wire_enum! {
    /// what kind of reply a question takes
    pub enum QKind {
        Single = "single",
        Multi = "multi",
        Text = "text",
        Number = "number",
        Order = "order",
    }
}

record! {
    /// an answer to choose: right or not, what it says when it was chosen
    pub struct Answer {
        pub text: Text,
        pub right: bool,
        pub explain: Text,
    }
}

record! {
    /// a question (see the module comment for what belongs to which kind)
    pub struct Question {
        /// "single", ...: QKind::parse; an unknown one is an issue
        pub kind: String,
        /// Markdown
        pub stem: Text,
        /// None: 1
        pub points: Option<u32>,
        /// Markdown
        pub explain: Text,
        pub answers: Vec<Answer>,
        /// the patterns of a text question
        pub accept: Vec<String>,
        /// a text question tells capitals apart
        pub case: bool,
        /// a number question: a JSON number or a text ("0,5")
        pub value: Value,
        pub tolerance: Value,
        /// an order question: the entries in the right order
        pub order: Vec<Text>,
    }
}

record! {
    /// academy/<kurs>/quiz/*.json
    pub struct Quiz {
        pub format: String,
        pub version: u32,
        pub title: Text,
        /// per cent to pass (0: 100)
        pub pass: u32,
        /// a test's time limit (0: none) and how many attempts it allows (0: as many as you like)
        pub minutes: u32,
        pub attempts: u32,
        pub shuffle: bool,
        pub questions: Vec<Question>,
    }
}

/// how a reply did
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marked {
    /// thousandths of a point: what it got, what it could get
    pub got: u32,
    pub max: u32,
    /// nothing is missing and nothing is too much
    pub right: bool,
}

/// the grade of a whole quiz
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Score {
    pub got: u64,
    pub total: u64,
    pub marks: Vec<Marked>,
}

impl Score {
    /// the points in per cent, rounded down (100 only when everything is right)
    pub fn percent(&self) -> u32 {
        if self.total == 0 { 0 } else { (self.got * 100 / self.total) as u32 }
    }
}

/// what a learner has given for a question; which part counts goes by its kind
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    /// single: the answer chosen (-1: none)
    pub choice: i32,
    /// multi: per answer 0 or 1 (as a check box keeps it)
    pub marks: Vec<i32>,
    /// text, number: what was typed
    pub text: String,
    /// order: the entries as they stand now (indices into `order`)
    pub order: Vec<usize>,
}

/// the order of an attempt, by the questions' places in the file
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub questions: Vec<usize>,
    /// per question: its answers (single, multi) or entries (order) as they are shown
    pub answers: Vec<Vec<usize>>,
}

/// how an attempt ended
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct End {
    /// the clock then (out of time: the limit, whenever it was noticed)
    pub at_us: u64,
    pub timeout: bool,
    pub score: Score,
    pub passed: bool,
}

/// a test being written: not kept across a restart
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attempt {
    pub seed: u64,
    pub plan: Plan,
    /// per question, by its place in the file
    pub replies: Vec<Reply>,
    pub started_us: u64,
    /// 0: no limit
    pub limit_us: u64,
    pub end: Option<End>,
}

// ---------------------------------------------------------------- numbers and order

/// a number in millionths: "254", "-0,5", "1.25"; None: it is none (or has more than six decimals)
pub fn fixed(s: &str) -> Option<i128> {
    let t = s.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t),
    };
    let (int, frac) = match t.split_once(['.', ',']) {
        Some((i, f)) => (i, f),
        None => (t, ""),
    };
    if (int.is_empty() && frac.is_empty()) || int.len() > 18 || frac.len() > 6 || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut v = if int.is_empty() { 0 } else { int.parse::<i128>().ok()? } * 1_000_000;
    if !frac.is_empty() {
        let mut f = frac.parse::<i128>().ok()?;
        for _ in frac.len()..6 {
            f *= 10;
        }
        v += f;
    }
    Some(if neg { -v } else { v })
}

// a JSON number or a text as a number
fn value_fixed(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) => fixed(n.as_str()),
        Value::String(s) => fixed(s),
        _ => None,
    }
}

// the next number of a small generator (splitmix64): an attempt's order, the same for the same seed on
// every system
fn next(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// a list in another order (Fisher and Yates)
fn shuffle(v: &mut [usize], s: &mut u64) {
    for i in (1..v.len()).rev() {
        let k = (next(s) % (i as u64 + 1)) as usize;
        v.swap(i, k);
    }
}

// the reply as a text question compares it: no spaces around it, runs of spaces as one
fn tidy(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

// a pattern's expression: it must match the whole reply
fn pattern(p: &str, case: bool) -> Result<Regex, String> {
    Regex::new(p, Syntax::Extended, !case).map_err(|e| String::from(e.text()))
}

// ---------------------------------------------------------------- questions

impl Question {
    /// its kind (None: unknown)
    pub fn kind_of(&self) -> Option<QKind> {
        QKind::parse(self.kind.trim())
    }

    /// the points it is worth
    pub fn worth(&self) -> u32 {
        self.points.unwrap_or(1)
    }

    /// nothing given yet; `shown`: an order question's entries as they stand at the start
    pub fn blank(&self, shown: &[usize]) -> Reply {
        Reply { choice: -1, marks: alloc::vec![0; self.answers.len()], text: String::new(), order: shown.to_vec() }
    }

    /// something was given (an order question always has its order)
    pub fn answered(&self, r: &Reply) -> bool {
        match self.kind_of() {
            Some(QKind::Single) => r.choice >= 0,
            Some(QKind::Multi) => r.marks.iter().any(|&m| m != 0),
            Some(QKind::Text | QKind::Number) => !r.text.trim().is_empty(),
            Some(QKind::Order) => true,
            None => false,
        }
    }

    /// what a reply gets
    pub fn grade(&self, r: &Reply) -> Marked {
        let max = self.worth().saturating_mul(MILLI);
        let all = |right: bool| Marked { got: if right { max } else { 0 }, max, right };
        match self.kind_of() {
            Some(QKind::Single) => all(usize::try_from(r.choice).ok().and_then(|i| self.answers.get(i)).is_some_and(|a| a.right)),
            Some(QKind::Multi) => {
                let rights = self.answers.iter().filter(|a| a.right).count() as u64;
                let chosen = |right: bool| self.answers.iter().zip(r.marks.iter()).filter(|(a, m)| a.right == right && **m != 0).count() as u64;
                let (good, bad) = (chosen(true), chosen(false));
                let got = if rights == 0 { 0 } else { max as u64 * good.saturating_sub(bad) / rights };
                Marked { got: got as u32, max, right: rights > 0 && good == rights && bad == 0 }
            }
            Some(QKind::Text) => {
                let t = tidy(&r.text);
                all(!t.is_empty() && self.accept.iter().filter_map(|p| pattern(p, self.case).ok()).any(|re| re.find(&t) == Some((0, t.len()))))
            }
            Some(QKind::Number) => {
                let tol = value_fixed(&self.tolerance).unwrap_or(0);
                all(matches!((fixed(&r.text), value_fixed(&self.value)), (Some(a), Some(b)) if (a - b).abs() <= tol))
            }
            Some(QKind::Order) => all(self.order.len() >= 2 && r.order.len() == self.order.len() && r.order.iter().enumerate().all(|(i, &k)| i == k)),
            None => all(false),
        }
    }

    /// a number question's value as text, as the file has it (for the result)
    pub fn value_text(&self) -> String {
        match &self.value {
            Value::Number(n) => n.as_str().to_string(),
            Value::String(s) => s.clone(),
            _ => String::new(),
        }
    }

    // what is wrong with question n
    fn issues(&self, n: usize) -> Vec<Problem> {
        let mut v = Vec::new();
        let Some(kind) = self.kind_of() else {
            v.push(fatal(msg!("Frage {}: die Art „{}“ gibt es nicht (single, multi, text, number, order).", n, self.kind)));
            return v;
        };
        if let Some(l) = missing(&self.stem) {
            v.push(fatal(msg!("Frage {}: der Text fehlt auf {}.", n, l)));
        }
        if self.worth() == 0 {
            v.push(fatal(msg!("Frage {} ist 0 Punkte wert.", n)));
        }
        if self.explain.de.trim().is_empty() && self.explain.en.trim().is_empty() {
            v.push(note(msg!("Frage {} hat keine Erklärung.", n)));
        } else if let Some(l) = missing(&self.explain) {
            v.push(fatal(msg!("Frage {}: die Erklärung fehlt auf {}.", n, l)));
        }
        match kind {
            QKind::Single | QKind::Multi => {
                if self.answers.len() < 2 {
                    v.push(fatal(msg!("Frage {} braucht mindestens zwei Antworten.", n)));
                }
                if self.answers.len() > ANSWERS_MAX {
                    v.push(fatal(msg!("Frage {} hat mehr als {} Antworten.", n, ANSWERS_MAX)));
                }
                let rights = self.answers.iter().filter(|a| a.right).count();
                if rights == 0 {
                    v.push(fatal(msg!("Frage {} hat keine richtige Antwort.", n)));
                }
                if kind == QKind::Single && rights > 1 {
                    v.push(fatal(msg!("Frage {} hat {} richtige Antworten; bei „single“ ist es genau eine.", n, rights)));
                }
                for (k, a) in self.answers.iter().enumerate() {
                    if let Some(l) = missing(&a.text) {
                        v.push(fatal(msg!("Frage {}, Antwort {}: der Text fehlt auf {}.", n, k + 1, l)));
                    }
                    if !(a.explain.de.trim().is_empty() && a.explain.en.trim().is_empty())
                        && let Some(l) = missing(&a.explain)
                    {
                        v.push(fatal(msg!("Frage {}, Antwort {}: die Erklärung fehlt auf {}.", n, k + 1, l)));
                    }
                }
            }
            QKind::Text => {
                if self.accept.is_empty() {
                    v.push(fatal(msg!("Frage {} nennt keine akzeptierten Muster (accept).", n)));
                }
                for p in &self.accept {
                    if let Err(why) = pattern(p, self.case) {
                        v.push(fatal(msg!("Frage {}: das Muster „{}“ lässt sich nicht übersetzen: {}", n, p, why)));
                    }
                }
            }
            QKind::Number => {
                if value_fixed(&self.value).is_none() {
                    v.push(fatal(msg!("Frage {}: der richtige Wert (value) ist keine Zahl.", n)));
                }
                if !matches!(self.tolerance, Value::Null) && value_fixed(&self.tolerance).is_none_or(|t| t < 0) {
                    v.push(fatal(msg!("Frage {}: die Toleranz (tolerance) ist keine Zahl ab 0.", n)));
                }
            }
            QKind::Order => {
                if self.order.len() < 2 {
                    v.push(fatal(msg!("Frage {} braucht mindestens zwei Einträge zum Ordnen.", n)));
                }
                if self.order.len() > ANSWERS_MAX {
                    v.push(fatal(msg!("Frage {} hat mehr als {} Einträge zum Ordnen.", n, ANSWERS_MAX)));
                }
                for (k, e) in self.order.iter().enumerate() {
                    if let Some(l) = missing(e) {
                        v.push(fatal(msg!("Frage {}, Eintrag {}: der Text fehlt auf {}.", n, k + 1, l)));
                    }
                }
            }
        }
        v
    }
}

// the language a text lacks, as a message
fn missing(t: &Text) -> Option<Msg> {
    if t.de.trim().is_empty() {
        Some(phrase("Deutsch"))
    } else if t.en.trim().is_empty() {
        Some(phrase("Englisch"))
    } else {
        None
    }
}

// ---------------------------------------------------------------- the file

impl Quiz {
    /// the text of a quiz's file -> the quiz, or why not
    pub fn from_text(text: &str) -> Result<Quiz, Msg> {
        let q = Quiz::from_json(&parse(text)?).ok_or_else(|| msg!("Die Datei enthält kein Quiz."))?;
        if q.format != QUIZ_FORMAT {
            return Err(msg!("Das ist keine Quizdatei."));
        }
        version_ok(q.version)?;
        Ok(q)
    }

    /// what is wrong in the file
    pub fn issues(&self) -> Vec<Problem> {
        let mut v = Vec::new();
        if self.questions.is_empty() {
            v.push(fatal(msg!("Das Quiz hat keine Fragen.")));
        }
        if self.questions.len() > QUESTIONS_MAX {
            v.push(fatal(msg!("Das Quiz hat mehr als {} Fragen.", QUESTIONS_MAX)));
        }
        if self.pass > 100 {
            v.push(fatal(msg!("Die Bestehensgrenze (pass) ist {}; höchstens 100 %.", self.pass)));
        }
        if !self.title.is_empty()
            && let Some(l) = missing(&self.title)
        {
            v.push(fatal(msg!("Der Titel des Quiz fehlt auf {}.", l)));
        }
        for (i, q) in self.questions.iter().enumerate() {
            v.extend(q.issues(i + 1));
        }
        v
    }

    /// the points there are, in thousandths
    pub fn total(&self) -> u64 {
        self.questions.iter().map(|q| q.worth() as u64 * MILLI as u64).sum()
    }

    /// the per cent that pass
    pub fn pass_percent(&self) -> u32 {
        if self.pass == 0 { 100 } else { self.pass }
    }

    /// the attempts left after `used` (None: no limit)
    pub fn attempts_left(&self, used: u32) -> Option<u32> {
        (self.attempts > 0).then(|| self.attempts.saturating_sub(used))
    }

    /// another attempt may start after `used`
    pub fn can_start(&self, used: u32) -> bool {
        self.attempts_left(used) != Some(0)
    }

    /// the order of an attempt: the same for the same seed.  Questions and answers stay as in the file
    /// without "shuffle"; the entries of an order question never start in the right order.
    pub fn plan(&self, seed: u64) -> Plan {
        let mut s = seed;
        let mut questions: Vec<usize> = (0..self.questions.len()).collect();
        if self.shuffle {
            shuffle(&mut questions, &mut s);
        }
        let answers = self
            .questions
            .iter()
            .map(|q| {
                if q.kind_of() == Some(QKind::Order) {
                    let mut v: Vec<usize> = (0..q.order.len()).collect();
                    shuffle(&mut v, &mut s);
                    if v.len() >= 2 && v.iter().enumerate().all(|(i, &k)| i == k) {
                        v.rotate_left(1);
                    }
                    v
                } else {
                    let mut v: Vec<usize> = (0..q.answers.len()).collect();
                    if self.shuffle {
                        shuffle(&mut v, &mut s);
                    }
                    v
                }
            })
            .collect();
        Plan { questions, answers }
    }

    /// every reply against its question
    pub fn grade(&self, replies: &[Reply]) -> Score {
        let marks: Vec<Marked> = self.questions.iter().zip(replies.iter()).map(|(q, r)| q.grade(r)).collect();
        Score { got: marks.iter().map(|m| m.got as u64).sum(), total: self.total(), marks }
    }
}

// ---------------------------------------------------------------- an attempt

impl Attempt {
    /// a new attempt from `clock` on: its order from the seed, no replies yet, the file's time limit
    pub fn new(q: &Quiz, seed: u64, clock: u64) -> Attempt {
        let plan = q.plan(seed);
        let replies = q.questions.iter().enumerate().map(|(i, x)| x.blank(if x.kind_of() == Some(QKind::Order) { &plan.answers[i] } else { &[] })).collect();
        Attempt { seed, plan, replies, started_us: clock, limit_us: q.minutes as u64 * 60_000_000, end: None }
    }

    /// what is left of the time limit (µs; None: there is none)
    pub fn remaining(&self, clock: u64) -> Option<u64> {
        if self.limit_us == 0 {
            return None;
        }
        let till = self.end.as_ref().map_or(clock, |e| e.at_us);
        Some((self.started_us + self.limit_us).saturating_sub(till))
    }

    /// the time is over and it was not handed in yet
    pub fn time_up(&self, clock: u64) -> bool {
        self.end.is_none() && self.limit_us > 0 && clock >= self.started_us + self.limit_us
    }

    /// how many questions have no reply
    pub fn unanswered(&self, q: &Quiz) -> usize {
        q.questions.iter().zip(self.replies.iter()).filter(|(x, r)| !x.answered(r)).count()
    }

    /// handed in, or out of time: graded once, then it stays; what it got
    pub fn finish(&mut self, q: &Quiz, clock: u64, timeout: bool) -> &End {
        let at_us = if timeout { self.started_us + self.limit_us } else { clock };
        let score = q.grade(&self.replies);
        let passed = score.percent() >= q.pass_percent();
        self.end.get_or_insert(End { at_us, timeout, score, passed })
    }
}
