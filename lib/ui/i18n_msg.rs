// i18n_msg -- messages with named, numbered and plural placeholders (a subset of ICU
// MessageFormat), so translations may reorder and inflect
//
//   msg(text, &[("n", 3.into()), ("dir", "Bilder".into())]) -> String
//
//   {name} {0} {}            an argument by name, by position, or the next one in order;
//                            numbers as they are, with the locale's decimal sign ("1234,5")
//   {n, number}              grouped ("1.234,5"); styles: integer (rounded), percent (a
//                            fraction: 0.25 -> "25 %"), currency/EUR, size (bytes: "1,5 MB")
//   {d, date} {d, date, long} {d, time} {d, datetime}   an Arg::Date (datetime::Civil)
//   {n, plural, =0 {keine Dateien} one {# Datei} other {# Dateien}}
//                            the CLDR category of the UI language (language_code()); =N
//                            matches before the categories; # is the number (grouped);
//                            "offset:1" before the options subtracts 1 for # and the category
//   {g, select, female {sie} male {er} other {es}}
//   '{' '}' '#' quote,  '' is an apostrophe; a lone apostrophe stays ("geht's")
//
// The numbers follow the user's format (locale()).  A missing argument prints as {name};
// broken syntax prints the rest of the text as it is.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use datetime::Civil;

use super::{DATE_LONG, DATE_SHORT, Locale, language_code, locale, plural};

/// one argument of msg()
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    Str(&'a str),
    Int(i64),
    Num(f64),
    Date(Civil),
}

impl<'a> From<&'a str> for Arg<'a> {
    fn from(s: &'a str) -> Self {
        Arg::Str(s)
    }
}
impl<'a> From<&'a String> for Arg<'a> {
    fn from(s: &'a String) -> Self {
        Arg::Str(s)
    }
}
impl From<i64> for Arg<'_> {
    fn from(n: i64) -> Self {
        Arg::Int(n)
    }
}
impl From<i32> for Arg<'_> {
    fn from(n: i32) -> Self {
        Arg::Int(n as i64)
    }
}
impl From<u32> for Arg<'_> {
    fn from(n: u32) -> Self {
        Arg::Int(n as i64)
    }
}
impl From<u64> for Arg<'_> {
    fn from(n: u64) -> Self {
        Arg::Int(n.min(i64::MAX as u64) as i64)
    }
}
impl From<usize> for Arg<'_> {
    fn from(n: usize) -> Self {
        Arg::Int(n.min(i64::MAX as usize) as i64)
    }
}
impl From<f64> for Arg<'_> {
    fn from(v: f64) -> Self {
        Arg::Num(v)
    }
}
impl From<f32> for Arg<'_> {
    fn from(v: f32) -> Self {
        Arg::Num(v as f64)
    }
}
impl From<Civil> for Arg<'_> {
    fn from(c: Civil) -> Self {
        Arg::Date(c)
    }
}

struct Cx<'a, 'b> {
    args: &'b [(&'b str, Arg<'a>)],
    loc: Locale,
    lang: String,
}

/// format a message (see the module comment)
pub fn msg(text: &str, args: &[(&str, Arg)]) -> String {
    let s: Vec<char> = text.chars().collect();
    let cx = Cx { args, loc: locale(), lang: language_code() };
    let mut out = String::with_capacity(text.len() + 16);
    let (mut p, mut seq) = (0, 0);
    run(&s, &mut p, &cx, &mut seq, None, false, &mut out);
    out
}

// text up to the end (or, nested, up to the '}' that closes it, which is not consumed)
fn run(s: &[char], p: &mut usize, cx: &Cx, seq: &mut usize, hash: Option<&str>, nested: bool, out: &mut String) {
    while *p < s.len() {
        let c = s[*p];
        match c {
            '\'' => quote(s, p, hash.is_some(), out),
            '{' => {
                let start = *p;
                *p += 1;
                if arg(s, p, cx, seq, out).is_none() {
                    out.extend(&s[start..]);
                    *p = s.len();
                }
            }
            '}' if nested => return,
            '#' if hash.is_some() => {
                out.push_str(hash.unwrap_or(""));
                *p += 1;
            }
            _ => {
                out.push(c);
                *p += 1;
            }
        }
    }
}

// an apostrophe: '' -> ', '{...' quoted up to the next lone apostrophe, else itself
fn quote(s: &[char], p: &mut usize, in_plural: bool, out: &mut String) {
    let next = s.get(*p + 1).copied();
    if next == Some('\'') {
        out.push('\'');
        *p += 2;
        return;
    }
    if !(matches!(next, Some('{' | '}')) || (in_plural && next == Some('#'))) {
        out.push('\'');
        *p += 1;
        return;
    }
    *p += 1;
    while *p < s.len() {
        if s[*p] == '\'' {
            if s.get(*p + 1) == Some(&'\'') {
                out.push('\'');
                *p += 2;
                continue;
            }
            *p += 1;
            return;
        }
        out.push(s[*p]);
        *p += 1;
    }
}

fn skip_ws(s: &[char], p: &mut usize) {
    while *p < s.len() && s[*p].is_whitespace() {
        *p += 1;
    }
}

// a word up to one of the stop characters, trimmed
fn word(s: &[char], p: &mut usize, stop: &[char]) -> String {
    let start = *p;
    while *p < s.len() && !stop.contains(&s[*p]) {
        *p += 1;
    }
    let w: String = s[start..*p].iter().collect();
    String::from(w.trim())
}

// the argument after '{' up to and with its '}' -> None on broken syntax
fn arg(s: &[char], p: &mut usize, cx: &Cx, seq: &mut usize, out: &mut String) -> Option<()> {
    let name = word(s, p, &[',', '}']);
    let value = lookup(cx, &name, seq);
    if s.get(*p) == Some(&'}') {
        *p += 1;
        match value {
            Some(v) => out.push_str(&plain(cx, v)),
            None => out.push_str(&format!("{{{name}}}")),
        }
        return Some(());
    }
    *p += 1; // ','
    let typ = word(s, p, &[',', '}']);
    let style = if s.get(*p) == Some(&',') {
        *p += 1;
        match typ.as_str() {
            "plural" | "select" => None,
            _ => Some(word(s, p, &['}'])),
        }
    } else {
        Some(String::new())
    };
    match typ.as_str() {
        "plural" | "select" if style.is_none() => {
            let opts = options(s, p)?;
            let n = value.map(|v| number(cx, v));
            let branch = if typ == "plural" { choose_plural(cx, &opts, n) } else { choose_select(&opts, value) }?;
            let hash = if typ == "plural" { n.map(|(x, digits)| hash_text(cx, x - opts.offset, digits)) } else { None };
            let (mut q, mut inner_seq) = (branch.0, *seq);
            let mut sub = String::new();
            run(&s[..branch.1], &mut q, cx, &mut inner_seq, hash.as_deref(), false, &mut sub);
            out.push_str(&sub);
            Some(())
        }
        _ => {
            if s.get(*p) != Some(&'}') {
                return None;
            }
            *p += 1;
            let style = style.unwrap_or_default();
            match value {
                Some(v) => out.push_str(&typed(cx, v, &typ, &style)),
                None => out.push_str(&format!("{{{name}}}")),
            }
            Some(())
        }
    }
}

fn lookup<'a>(cx: &Cx<'a, '_>, name: &str, seq: &mut usize) -> Option<Arg<'a>> {
    if name.is_empty() {
        *seq += 1;
        return cx.args.get(*seq - 1).map(|a| a.1);
    }
    if name.bytes().all(|b| b.is_ascii_digit()) {
        return name.parse::<usize>().ok().and_then(|k| cx.args.get(k)).map(|a| a.1);
    }
    cx.args.iter().find(|a| a.0 == name).map(|a| a.1)
}

// "offset:N"? then "selector {message}" pairs up to the closing '}' -> (offset, [(selector,
// start, end)]); the message is s[start..end]
struct Opts {
    offset: f64,
    list: Vec<(String, usize, usize)>,
}

fn options(s: &[char], p: &mut usize) -> Option<Opts> {
    let mut o = Opts { offset: 0.0, list: Vec::new() };
    loop {
        skip_ws(s, p);
        match s.get(*p) {
            None => return None,
            Some('}') => {
                *p += 1;
                return Some(o);
            }
            _ => {}
        }
        let sel = word(s, p, &['{', ' ', '\t', '\r', '\n', '}']);
        if let Some(n) = sel.strip_prefix("offset:") {
            o.offset = n.trim().parse().ok()?;
            continue;
        }
        skip_ws(s, p);
        if sel.is_empty() || s.get(*p) != Some(&'{') {
            return None;
        }
        *p += 1;
        let start = *p;
        close(s, p)?;
        o.list.push((sel, start, *p - 1));
    }
}

// past the '}' that closes a message (nesting and quotes respected)
fn close(s: &[char], p: &mut usize) -> Option<()> {
    let mut depth = 0;
    while *p < s.len() {
        match s[*p] {
            '\'' => {
                let mut sink = String::new();
                quote(s, p, true, &mut sink);
                continue;
            }
            '{' => depth += 1,
            '}' if depth == 0 => {
                *p += 1;
                return Some(());
            }
            '}' => depth -= 1,
            _ => {}
        }
        *p += 1;
    }
    None
}

// a numeric argument -> (value, visible fraction digits)
fn number(cx: &Cx, v: Arg) -> (f64, u32) {
    match v {
        Arg::Int(n) => (n as f64, 0),
        Arg::Num(x) => {
            let s = cx.loc.num_plain(x);
            (x, s.split(cx.loc.decimal()).nth(1).map_or(0, |f| f.chars().count() as u32))
        }
        Arg::Str(t) => {
            let t = t.trim();
            let digits = t.split(['.', ',']).nth(1).map_or(0, |f| f.len() as u32);
            (t.replace(',', ".").parse().unwrap_or(f64::NAN), digits)
        }
        Arg::Date(_) => (f64::NAN, 0),
    }
}

fn hash_text(cx: &Cx, x: f64, digits: u32) -> String {
    if x.is_nan() { String::new() } else { cx.loc.fmt_num(x, digits) }
}

fn choose_plural(cx: &Cx, o: &Opts, n: Option<(f64, u32)>) -> Option<(usize, usize)> {
    let pick = |name: &str| o.list.iter().find(|e| e.0 == name).map(|e| (e.1, e.2));
    if let Some((x, digits)) = n {
        // =N on the value itself
        if let Some(e) = o.list.iter().find(|e| e.0.strip_prefix('=').and_then(|v| v.parse::<f64>().ok()) == Some(x)) {
            return Some((e.1, e.2));
        }
        let y = x - o.offset;
        if !y.is_nan() {
            let a = if y < 0.0 { -y } else { y };
            let (i, v, f) = operands(cx, a, digits);
            if let Some(b) = pick(plural(&cx.lang, i, v, f).name()) {
                return Some(b);
            }
        }
    }
    pick("other")
}

// CLDR operands of a non-negative number shown with `digits` fraction digits
fn operands(cx: &Cx, a: f64, digits: u32) -> (u64, u32, u64) {
    let s = cx.loc.num(a, digits, false);
    let (int, frac) = s.split_once(cx.loc.decimal()).unwrap_or((&s, ""));
    (int.parse().unwrap_or(u64::MAX), frac.len() as u32, frac.parse().unwrap_or(0))
}

fn choose_select(o: &Opts, v: Option<Arg>) -> Option<(usize, usize)> {
    let key = match v {
        Some(Arg::Str(s)) => Some(String::from(s)),
        Some(Arg::Int(n)) => Some(format!("{n}")),
        _ => None,
    };
    key.and_then(|k| o.list.iter().find(|e| e.0 == k)).or_else(|| o.list.iter().find(|e| e.0 == "other")).map(|e| (e.1, e.2))
}

// {x}: as it is
fn plain(cx: &Cx, v: Arg) -> String {
    match v {
        Arg::Str(s) => String::from(s),
        Arg::Int(n) => format!("{n}"),
        Arg::Num(x) => cx.loc.num_plain(x),
        Arg::Date(c) => cx.loc.fmt_date(&c, DATE_SHORT),
    }
}

// {x, number|date|time|datetime, style}
fn typed(cx: &Cx, v: Arg, typ: &str, style: &str) -> String {
    let style = style.trim().trim_start_matches("::");
    match (typ, v) {
        ("date", Arg::Date(c)) => cx.loc.fmt_date(&c, if style == "long" || style == "full" { DATE_LONG } else { DATE_SHORT }),
        ("time", Arg::Date(c)) => cx.loc.fmt_time(&c),
        ("datetime", Arg::Date(c)) => cx.loc.fmt_date_time(&c),
        ("number", Arg::Int(_) | Arg::Num(_) | Arg::Str(_)) => {
            let (x, digits) = number(cx, v);
            if x.is_nan() {
                return plain(cx, v);
            }
            if style == "integer" {
                cx.loc.fmt_num(x, 0)
            } else if style == "percent" {
                cx.loc.fmt_percent(x, 0)
            } else if let Some(code) = style.strip_prefix("currency/") {
                cx.loc.fmt_currency(x, code.trim())
            } else if style == "size" {
                cx.loc.fmt_size(if x > 0.0 { x as u64 } else { 0 })
            } else {
                cx.loc.fmt_num(x, digits)
            }
        }
        _ => plain(cx, v),
    }
}
