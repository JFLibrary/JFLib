// css -- the style attribute and <style> sheets: declarations and simple selectors
//
//   decls(text) -> [(name, value)]        "fill: red; stroke-width: 2" (comments dropped,
//                                         !important ignored, names lower case)
//   sheet(text, order, rules)             the rules of a style sheet; at-rules (@media,
//                                         @font-face, @import) are skipped with their blocks
//   Sel::matches(dom, el)                 does the selector match element el
//
// Selectors: lists (a, b) of compound selectors -- `*`, a type, `.class`, `#id`, `[attr]`,
// `[attr=value]` -- joined by descendant (blank) and child (`>`) combinators.  A selector
// with anything else (pseudo-classes, `+`, `~`) never matches.  Specificity: ids, classes /
// attributes, types; equal ones in sheet order.

use alloc::string::String;
use alloc::vec::Vec;

use crate::xml::{Dom, NONE};

const MAX_RULES: usize = 4096;
const MAX_PARTS: usize = 16; // compounds per selector
const MATCH_STEPS: u32 = 4096; // compound tests per selector and element

#[derive(Clone, Debug, Default)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
}

/// a complex selector, right to left: (compound, child combinator to the next one left)
#[derive(Clone, Debug)]
pub(crate) struct Sel {
    parts: Vec<(Compound, bool)>,
    pub spec: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Rule {
    pub sel: Sel,
    pub order: u32,
    pub decls: Vec<(String, String)>,
}

fn strip_comments(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut rest = t;
    while let Some(k) = rest.find("/*") {
        out.push_str(&rest[..k]);
        match rest[k + 2..].find("*/") {
            Some(e) => rest = &rest[k + 2 + e + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// the declarations of a block / style attribute
pub(crate) fn decls(text: &str) -> Vec<(String, String)> {
    let t = strip_comments(text);
    let mut out = Vec::new();
    for d in split_decls(&t) {
        let Some(c) = d.find(':') else { continue };
        let name = d[..c].trim().to_ascii_lowercase();
        let mut val = d[c + 1..].trim();
        if let Some(k) = val.to_ascii_lowercase().find("!important") {
            val = val[..k].trim();
        }
        if !name.is_empty() && !val.is_empty() {
            out.push((name, String::from(val)));
        }
    }
    out
}

// split at ';' outside quotes and parentheses (url(data:...;base64,...))
fn split_decls(t: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = t.as_bytes();
    let (mut depth, mut quote, mut start) = (0i32, 0u8, 0usize);
    for (i, &c) in b.iter().enumerate() {
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => quote = c,
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth <= 0 => {
                out.push(&t[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&t[start..]);
    out
}

fn ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c >= 0x80
}

// one compound selector at s[*i..] (None: unsupported)
fn compound(s: &str, i: &mut usize) -> Option<Compound> {
    let b = s.as_bytes();
    let mut c = Compound::default();
    let mut any = false;
    let ident = |i: &mut usize| -> String {
        let st = *i;
        while *i < b.len() && (ident_char(b[*i]) || b[*i] == b'\\') {
            *i += 1;
        }
        String::from(&s[st..*i])
    };
    while *i < b.len() {
        match b[*i] {
            b'*' => {
                *i += 1;
                any = true;
            }
            b'.' => {
                *i += 1;
                let n = ident(i);
                if n.is_empty() {
                    return None;
                }
                c.classes.push(n);
                any = true;
            }
            b'#' => {
                *i += 1;
                let n = ident(i);
                if n.is_empty() {
                    return None;
                }
                c.id = Some(n);
                any = true;
            }
            b'[' => {
                let end = s[*i..].find(']')? + *i;
                let inner = &s[*i + 1..end];
                *i = end + 1;
                match inner.find('=') {
                    Some(k) => {
                        let (n, v) = (inner[..k].trim(), inner[k + 1..].trim().trim_matches(|c| c == '"' || c == '\''));
                        if n.ends_with(['~', '|', '^', '$', '*']) {
                            return None;
                        }
                        c.attrs.push((String::from(n), Some(String::from(v))));
                    }
                    None => c.attrs.push((String::from(inner.trim()), None)),
                }
                any = true;
            }
            ch if ident_char(ch) => {
                if any {
                    return None; // a type after other parts
                }
                c.tag = Some(ident(i));
                any = true;
            }
            b':' => return None,
            _ => break,
        }
    }
    if any { Some(c) } else { None }
}

// a complex selector (None: unsupported)
fn selector(s: &str) -> Option<Sel> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut left_to_right: Vec<(Compound, bool)> = Vec::new(); // (compound, joined to the previous by '>')
    let mut child = false;
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b'>' {
            if left_to_right.is_empty() || child {
                return None;
            }
            child = true;
            i += 1;
            continue;
        }
        if b[i] == b'+' || b[i] == b'~' {
            return None;
        }
        let c = compound(s, &mut i)?;
        left_to_right.push((c, child));
        child = false;
    }
    if left_to_right.is_empty() || child || left_to_right.len() > MAX_PARTS {
        return None;
    }
    let (mut ids, mut cls, mut tags) = (0u32, 0u32, 0u32);
    for (c, _) in &left_to_right {
        ids += c.id.is_some() as u32;
        cls += (c.classes.len() + c.attrs.len()) as u32;
        tags += c.tag.is_some() as u32;
    }
    // right to left; the flag says how the part is joined to the one on its left
    let n = left_to_right.len();
    let mut parts = Vec::with_capacity(n);
    for k in (0..n).rev() {
        let joined_child = left_to_right[k].1;
        parts.push((left_to_right[k].0.clone(), joined_child));
    }
    Some(Sel { parts, spec: ids.min(255) << 16 | cls.min(255) << 8 | tags.min(255) })
}

/// the rules of a style sheet, appended with increasing order numbers
pub(crate) fn sheet(text: &str, order: &mut u32, rules: &mut Vec<Rule>) {
    let t = strip_comments(text);
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b';') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        // the prelude up to '{' (or ';' for a statement at-rule)
        let p0 = i;
        while i < b.len() && b[i] != b'{' && !(b[i] == b';' && b[p0] == b'@') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b';' {
            i += 1; // @import ...;
            continue;
        }
        let prelude = &t[p0..i];
        // the block, nested braces included
        let mut depth = 0;
        let b0 = i + 1;
        while i < b.len() {
            if b[i] == b'{' {
                depth += 1;
            } else if b[i] == b'}' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            i += 1;
        }
        let block = &t[b0..i.min(b.len())];
        i += 1;
        if prelude.trim_start().starts_with('@') {
            continue;
        }
        let ds = decls(block);
        if ds.is_empty() {
            continue;
        }
        for one in prelude.split(',') {
            if rules.len() >= MAX_RULES {
                return;
            }
            if let Some(sel) = selector(one) {
                *order += 1;
                rules.push(Rule { sel, order: *order, decls: ds.clone() });
            }
        }
    }
}

fn has_class(list: Option<&str>, c: &str) -> bool {
    list.is_some_and(|l| l.split_ascii_whitespace().any(|x| x == c))
}

fn compound_matches(c: &Compound, dom: &Dom, el: u32) -> bool {
    let Some(e) = dom.els.get(el as usize) else { return false };
    if let Some(t) = &c.tag {
        if *t != e.name {
            return false;
        }
    }
    if let Some(id) = &c.id {
        if e.attr("id") != Some(id.as_str()) {
            return false;
        }
    }
    let cl = e.attr("class");
    if !c.classes.iter().all(|x| has_class(cl, x)) {
        return false;
    }
    c.attrs.iter().all(|(n, v)| match (e.attr(n), v) {
        (Some(_), None) => true,
        (Some(a), Some(v)) => a == v,
        _ => false,
    })
}

impl Sel {
    /// does the selector match element el of dom (a pathological selector gives up after
    /// MATCH_STEPS compound tests: no match); total counts the tests of the whole cascade
    pub(crate) fn matches(&self, dom: &Dom, el: u32, total: &mut u32) -> bool {
        let mut budget = MATCH_STEPS.min(*total);
        let start = budget;
        let r = self.match_from(dom, 0, el, &mut budget);
        *total -= start - budget;
        r
    }

    fn match_from(&self, dom: &Dom, k: usize, el: u32, budget: &mut u32) -> bool {
        let Some((c, child)) = self.parts.get(k) else { return true };
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        if !compound_matches(c, dom, el) {
            return false;
        }
        if k + 1 >= self.parts.len() {
            return true;
        }
        let mut p = dom.els.get(el as usize).map_or(NONE, |e| e.parent);
        if *child {
            return p != NONE && self.match_from(dom, k + 1, p, budget);
        }
        while p != NONE && *budget > 0 {
            if self.match_from(dom, k + 1, p, budget) {
                return true;
            }
            p = dom.els.get(p as usize).map_or(NONE, |e| e.parent);
        }
        false
    }
}
