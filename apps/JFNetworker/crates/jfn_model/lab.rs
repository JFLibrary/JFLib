// lab -- the task of a lab project: a title, an introduction, goals that checks in the .scn
// expectation syntax decide, hints, every text in German and English
//
//   Lab { title, summary, topic, intro, ordered, goals, hints }   Project::lab (None: a free
//                                                 project); the "lab" member of a .jfnet file
//   Goal { text, checks, hints }                  checks: the words of "expect ..." lines of a
//                                                 scenario (scenario.rs), all of them must hold
//                                                 at one moment
//   Text { de, en } / text.get(en)                a text in both languages (English falls back
//                                                 to the German)
//   lab.issues(&project) -> Vec<Msg>              what is wrong with it: no title or goals, a
//                                                 goal without checks, a check that does not
//                                                 parse, a device the project lacks
//   Goal::parse_checks() -> Result<Vec<Expect>, (index, why)>
//
// A text in a file is a string (German, shown in every language) or a pair ["Deutsch",
// "English"].  Which goals are reached, and when, is the simulator's business (jfn_sim's goal.rs
// evaluates the checks against the running engine); the file only describes the task.

use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::{Field, Value};
use agent_core::record;

use crate::msg;
use crate::msg::Msg;
use crate::project::Project;
use crate::scenario::{self, Expect};

/// a text in German and English (the English may be missing: the German stands in)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub de: String,
    pub en: String,
}

impl Text {
    pub fn new(de: &str, en: &str) -> Text {
        Text { de: String::from(de), en: String::from(en) }
    }
    /// the text in English (`en`) or German
    pub fn get(&self, en: bool) -> &str {
        if en && !self.en.is_empty() { &self.en } else { &self.de }
    }
    pub fn is_empty(&self) -> bool {
        self.de.trim().is_empty()
    }
}

/// a string (German only) or a pair ["Deutsch", "English"]; {"de": .., "en": ..} reads too
impl Field for Text {
    fn put(&self) -> Value {
        if self.en.is_empty() {
            Value::string(self.de.as_str())
        } else {
            Value::Array(alloc::vec![Value::string(self.de.as_str()), Value::string(self.en.as_str())])
        }
    }
    fn take(v: Option<&Value>) -> Self {
        let s = |x: Option<&Value>| String::from(x.and_then(Value::as_str).unwrap_or(""));
        match v {
            Some(Value::String(t)) => Text { de: t.clone(), en: String::new() },
            Some(Value::Array(a)) => Text { de: s(a.first()), en: s(a.get(1)) },
            Some(o @ Value::Object(_)) => Text { de: s(o.get("de")), en: s(o.get("en")) },
            _ => Text::default(),
        }
    }
}

record! {
    /// a goal of a lab: what the user should reach, the checks that say it is reached, hints
    /// that open one by one
    pub struct Goal {
        pub text: Text,
        /// expectations as a scenario writes them after "expect" ("LB1 svc lb metric
        /// backends_up == 3", "WEB3 route 10.0.2.0/24 via connected"); a leading "expect" is
        /// allowed; all of them hold at one moment when the goal is reached
        pub checks: Vec<String>,
        pub hints: Vec<Text>,
    }
}

record! {
    /// the task of a lab project
    pub struct Lab {
        pub title: Text,
        /// one line for the list of labs
        pub summary: Text,
        /// the shelf of the list: "net" (networks) or "ds" (distributed systems)
        pub topic: String,
        /// the introduction, a text per paragraph
        pub intro: Vec<Text>,
        /// a goal counts only after the goals before it (a story: change, break, recover)
        pub ordered: bool,
        pub goals: Vec<Goal>,
        /// hints for the whole task, opened one by one
        pub hints: Vec<Text>,
    }
}

impl Goal {
    /// the checks as expectations; Err: the first that does not parse (its index, why)
    pub fn parse_checks(&self) -> Result<Vec<Expect>, (usize, String)> {
        self.checks.iter().enumerate().map(|(i, c)| scenario::parse_expectation(c).map_err(|e| (i, e))).collect()
    }
}

impl Lab {
    /// what is wrong with the lab in this project (German messages for the app's catalogue)
    pub fn issues(&self, p: &Project) -> Vec<Msg> {
        let mut v = Vec::new();
        if self.title.is_empty() {
            v.push(msg!("Das Labor hat keinen Titel."));
        }
        if self.goals.is_empty() {
            v.push(msg!("Das Labor hat keine Ziele."));
        }
        for (i, g) in self.goals.iter().enumerate() {
            let n = i + 1;
            if g.text.is_empty() {
                v.push(msg!("Ziel {} hat keinen Text.", n));
            }
            if g.checks.is_empty() {
                v.push(msg!("Ziel {} hat keine Prüfung.", n));
            }
            for c in &g.checks {
                match scenario::parse_expectation(c) {
                    Err(e) => v.push(msg!("Ziel {}: \u{201e}{}\u{201c} ist keine Prüfung: {}", n, c, e)),
                    Ok(x) => {
                        if let Some(d) = x.dev()
                            && p.find(d).is_none()
                        {
                            v.push(msg!("Ziel {}: das Gerät {} gibt es im Projekt nicht.", n, d));
                        }
                    }
                }
            }
        }
        v
    }
}
