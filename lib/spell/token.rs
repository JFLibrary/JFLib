// token -- the words of a text that are worth checking
//
// A word is a run of letters (alphabetic characters and combining marks, plus the
// dictionary's WORDCHARS); apostrophes and hyphens join letters ("don't", "E-Mail"), dots only
// when WORDCHARS has them ("z.B").  Skipped: words with a digit ("3D", "MP3", "1st"), and every
// blank-separated chunk that looks like a URL, an e-mail address or a path (contains "://",
// '@' or '\\', starts with "www.", '/' or "~/", or a drive "C:").  A token records whether a
// '.' follows it, so an abbreviation ("usw.") can be checked with its dot.

use alloc::vec::Vec;

/// a word of a text: its byte range, and whether a '.' follows it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub dot: bool,
}

/// a combining mark (the common blocks; core has no general category)
fn is_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200C..=0x200D | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F)
}

fn is_letter(c: char, wordchars: &[char]) -> bool {
    (c.is_alphabetic() || is_mark(c) || c.is_numeric() || wordchars.contains(&c)) && !is_joiner(c, wordchars)
}

// characters that belong to a word only between two letters
fn is_joiner(c: char, wordchars: &[char]) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{02BC}' | '-' | '\u{2010}' | '\u{2011}') || ((c == '.' || c == '\u{00B7}') && wordchars.contains(&c))
}

/// a blank-separated chunk that is no prose: a URL, an e-mail address, a path
fn no_prose(chunk: &str) -> bool {
    let t = chunk.trim_start_matches(|c: char| matches!(c, '(' | '[' | '{' | '<' | '"' | '\'' | '\u{201E}' | '\u{201C}' | '\u{00AB}'));
    let b = t.as_bytes();
    t.contains("://")
        || t.contains('@')
        || t.contains('\\')
        || (b.len() >= 4 && b[..4].eq_ignore_ascii_case(b"www."))
        || t.starts_with('/')
        || t.starts_with("~/")
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'/' || b[2] == b'\\'))
}

/// the words of `text` to check (see the module header); `wordchars`: the dictionary's
/// WORDCHARS
pub fn tokens(text: &str, wordchars: &[char]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        // the next chunk between blanks
        let rest = &text[i..];
        let Some(s) = rest.find(|c: char| !c.is_whitespace()) else { break };
        let cs = i + s;
        let ce = text[cs..].find(char::is_whitespace).map_or(text.len(), |e| cs + e);
        if !no_prose(&text[cs..ce]) {
            words_in(text, cs, ce, wordchars, &mut out);
        }
        i = ce;
    }
    out
}

// the words of the chunk text[cs..ce]
fn words_in(text: &str, cs: usize, ce: usize, wordchars: &[char], out: &mut Vec<Token>) {
    let chunk = &text[cs..ce];
    let cv: Vec<(usize, char)> = chunk.char_indices().collect();
    let mut k = 0;
    while k < cv.len() {
        if !is_letter(cv[k].1, wordchars) {
            k += 1;
            continue;
        }
        let start = k;
        let mut digit = false;
        while k < cv.len() {
            let c = cv[k].1;
            if is_letter(c, wordchars) {
                digit |= c.is_numeric();
                k += 1;
            } else if is_joiner(c, wordchars) && k + 1 < cv.len() && is_letter(cv[k + 1].1, wordchars) {
                k += 1;
            } else {
                break;
            }
        }
        let s = cs + cv[start].0;
        let e = if k < cv.len() { cs + cv[k].0 } else { ce };
        if !digit {
            out.push(Token { start: s, end: e, dot: text[e..].starts_with('.') });
        }
    }
}
