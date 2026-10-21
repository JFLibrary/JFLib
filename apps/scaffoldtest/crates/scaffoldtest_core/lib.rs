// scaffoldtest_core -- what the window and the command line program of scaffoldtest share
//
// An app-local crate, built and linted like the library crates (docs/GUIDE.md section 16):
//   count(text) -> Counts        lines, words and characters
//   summary(&Counts) -> String   "2 Zeilen, 3 Wörter, 14 Zeichen"
//   Counts::add                  sums
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

/// what count() finds in a text
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// lines: the line breaks, plus one for a last line without a break
    pub lines: usize,
    /// words: runs of characters that are no white space
    pub words: usize,
    /// characters (Unicode scalar values)
    pub chars: usize,
}

impl Counts {
    /// add the counts of another text
    pub fn add(&mut self, o: &Counts) {
        self.lines += o.lines;
        self.words += o.words;
        self.chars += o.chars;
    }
}

/// the lines, words and characters of a text
pub fn count(text: &str) -> Counts {
    let mut c = Counts::default();
    let mut in_word = false;
    for ch in text.chars() {
        c.chars += 1;
        if ch == '\n' {
            c.lines += 1;
        }
        let space = ch.is_whitespace();
        if !space && !in_word {
            c.words += 1;
        }
        in_word = !space;
    }
    if !text.is_empty() && !text.ends_with('\n') {
        c.lines += 1;
    }
    c
}

/// the counts as one German line
pub fn summary(c: &Counts) -> String {
    let n = |k: usize, one: &str, many: &str| format!("{} {}", k, if k == 1 { one } else { many });
    format!("{}, {}, {}", n(c.lines, "Zeile", "Zeilen"), n(c.words, "Wort", "Wörter"), n(c.chars, "Zeichen", "Zeichen"))
}
