// msg -- texts of the model and the simulator for the app's catalogue: a German template with
// placeholders and its arguments, so the app can show them in its language
//
//   Msg { tpl, args }          tpl: the German template (the key of the catalogue,
//                              apps/JFNetworker/src/locales/engine.tsv), args: the values,
//                              formatted already
//   msg!("… {} …", a, b)       a Msg; the arguments are formatted with Display, a Msg argument
//                              stays a message of its own and is translated with the outer
//                              one; the template is checked against the arguments when it
//                              compiles (as by format!)
//   Msg::new(tpl, args)        from its parts;  Msg::plain(text): a text no catalogue knows
//                              (another layer's error, a name)
//   text() / Display           the German text
//   text_with(&tr)             the text in the app's language: tr maps a German template to
//                              its translation (the app passes its t()), nested messages
//                              included
//   phrase(p)                  a fixed term as a message ("Konsole", "Kupfer, gerade")
//   dur(ns)                    a span of time: "1,250 s", "3,200 ms", "450 µs", "12 ns" in
//                              German (as fmt_ns of the trace), "1.250 s" in English
//   list(items)                items joined by "{}, {}" (a separator a language may change)
//
// Placeholders: "{}" takes the next argument, "{0}" .. "{9}" a given one, so a translation may
// reorder them ("{}" counts only the "{}" before it, as in format!); "{{" and "}}" are braces.
// There are no formats like "{:03}": such values are formatted before.  An argument the
// template leaves out is dropped, a placeholder without one stays empty.
//
// A nested message travels in its argument as text: U+0001 and its template, then per
// argument U+0002, the argument's length in bytes, U+0003 and the argument.  Msg::arg (which
// msg! calls) registers the templates of nested messages, so text_with can give the app's
// lookup the 'static key it needs.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// a text of the model or the simulator: a German template with placeholders and its
/// arguments; the app looks the template up in its catalogue and fills it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Msg {
    pub tpl: &'static str,
    pub args: Vec<String>,
}

// a translation: German template -> the template in the app's language
type Tr<'a> = &'a dyn Fn(&'static str) -> &'static str;

// the templates of nested messages (text_with hands them to the app's lookup, which takes
// 'static keys only)
static NESTED: rt::sync::SpinLock<BTreeSet<&'static str>> = rt::sync::SpinLock::new(BTreeSet::new());

const NEST: char = '\u{1}';
const ARG: char = '\u{2}';
const LEN: char = '\u{3}';

impl Msg {
    pub fn new(tpl: &'static str, args: Vec<String>) -> Msg {
        Msg { tpl, args }
    }
    /// a text that is not translated (an error of another layer, a name)
    pub fn plain(text: impl Into<String>) -> Msg {
        Msg { tpl: "{}", args: alloc::vec![text.into()] }
    }
    /// no text at all
    pub fn is_empty(&self) -> bool {
        self.tpl.is_empty()
    }
    /// the German text
    pub fn text(&self) -> String {
        render(self.tpl, &self.args, None)
    }
    /// the text with the template translated by `tr` (the app's catalogue), and the templates
    /// of the messages inside it
    pub fn text_with(&self, tr: &dyn Fn(&'static str) -> &'static str) -> String {
        render(tr(self.tpl), &self.args, Some(tr))
    }
    /// this message as the argument of another one, translated with it (msg! does this for
    /// Msg arguments)
    pub fn arg(&self) -> String {
        NESTED.lock().insert(self.tpl);
        let mut s = String::with_capacity(self.tpl.len() + 8 + self.args.iter().map(|a| a.len() + 6).sum::<usize>());
        s.push(NEST);
        s.push_str(self.tpl);
        for a in &self.args {
            s.push(ARG);
            s.push_str(&a.len().to_string());
            s.push(LEN);
            s.push_str(a);
        }
        s
    }
}

impl fmt::Display for Msg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.text())
    }
}

/// a fixed term as a message: a name of the catalogue ("Konsole"), a reason ("ungültiger
/// Name"); the catalogue translates it like any template
pub fn phrase(p: &'static str) -> Msg {
    Msg { tpl: p, args: Vec::new() }
}

/// a span of time as people read it, with a sign: "1,250 s", "3,200 ms", "450 µs", "12 ns"
/// (German, as fmt_ns of the trace); the catalogue gives other languages their decimal mark
pub fn dur(ns: i64) -> Msg {
    let (sign, a) = if ns < 0 { ("-", ns.unsigned_abs()) } else { ("", ns as u64) };
    if a >= 1_000_000_000 {
        crate::msg!("{},{} s", format!("{}{}", sign, a / 1_000_000_000), format!("{:03}", a % 1_000_000_000 / 1_000_000))
    } else if a >= 1_000_000 {
        crate::msg!("{},{} ms", format!("{}{}", sign, a / 1_000_000), format!("{:03}", a % 1_000_000 / 1000))
    } else if a >= 1000 {
        crate::msg!("{} \u{b5}s", format!("{}{}", sign, a / 1000))
    } else {
        crate::msg!("{} ns", format!("{}{}", sign, a))
    }
}

/// items joined by ", " ("{}, {}" in the catalogue); nothing for no items
pub fn list(items: Vec<Msg>) -> Msg {
    let mut it = items.into_iter().rev();
    let Some(mut acc) = it.next() else {
        return Msg::default();
    };
    for m in it {
        acc = crate::msg!("{}, {}", m, acc);
    }
    acc
}

// a nested message in an argument: its template and arguments (None: a plain argument)
fn nested(a: &str) -> Option<(&str, Vec<&str>)> {
    let rest = a.strip_prefix(NEST)?;
    let (tpl, mut rest) = match rest.find(ARG) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let mut args = Vec::new();
    while let Some(r) = rest.strip_prefix(ARG) {
        let i = r.find(LEN)?;
        let n: usize = r[..i].parse().ok()?;
        args.push(r.get(i + 1..i + 1 + n)?);
        rest = &r[i + 1 + n..];
    }
    rest.is_empty().then_some((tpl, args))
}

// an argument as text: a nested message rendered (and translated), a plain one as it is
fn arg_text(a: &str, tr: Option<Tr<'_>>, out: &mut String) {
    if !a.starts_with(NEST) {
        out.push_str(a);
        return;
    }
    match nested(a) {
        Some((tpl, args)) => {
            let key = tr.and_then(|f| {
                let k = NESTED.lock().get(tpl).copied();
                k.map(f)
            });
            out.push_str(&render(key.unwrap_or(tpl), &args, tr));
        }
        // not what Msg::arg makes: the text without the control characters
        None => out.extend(a.chars().filter(|c| !matches!(*c, NEST | ARG | LEN))),
    }
}

// the template with its placeholders filled
fn render<S: AsRef<str>>(tpl: &str, args: &[S], tr: Option<Tr<'_>>) -> String {
    let mut out = String::with_capacity(tpl.len() + 16 * args.len());
    let mut next = 0usize;
    let mut rest = tpl;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let r = &rest[i..];
        if r.starts_with("{{") || r.starts_with("}}") {
            out.push_str(&r[..1]);
            rest = &r[2..];
            continue;
        }
        if r.starts_with('}') {
            out.push('}');
            rest = &r[1..];
            continue;
        }
        let Some(j) = r[1..].find('}') else {
            out.push_str(r);
            rest = "";
            break;
        };
        let inner = &r[1..1 + j];
        let k = if inner.is_empty() {
            next += 1;
            Some(next - 1)
        } else if inner.bytes().all(|b| b.is_ascii_digit()) {
            inner.parse::<usize>().ok()
        } else {
            None
        };
        match k {
            Some(k) => {
                if let Some(a) = args.get(k) {
                    arg_text(a.as_ref(), tr, &mut out);
                }
            }
            // no placeholder of ours: as it is
            None => out.push_str(&r[..j + 2]),
        }
        rest = &r[j + 2..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- what msg! needs

/// a message from a template and its arguments: `msg!("Port {} ist schon belegt.", port)`;
/// a Msg argument stays a message (its template is translated too)
#[macro_export]
macro_rules! msg {
    ($tpl:literal $(, $a:expr)* $(,)?) => {{
        // the template against its arguments, as format! checks them (never runs)
        if false {
            let _ = ::core::format_args!($tpl $(, $a)*);
        }
        #[allow(unused_imports)]
        use $crate::msg::{__ArgMsg as _, __ArgText as _};
        $crate::msg::Msg::new($tpl, $crate::msg::__args([$((&$crate::msg::__Arg(&$a)).__arg()),*]))
    }};
}

#[doc(hidden)]
pub struct __Arg<'a, T: ?Sized>(pub &'a T);

// a Msg argument becomes a nested message, anything else its Display text (the impl on the
// wrapper itself wins over the one on a reference to it)
#[doc(hidden)]
pub trait __ArgMsg {
    fn __arg(&self) -> String;
}

impl __ArgMsg for __Arg<'_, Msg> {
    fn __arg(&self) -> String {
        self.0.arg()
    }
}

impl __ArgMsg for __Arg<'_, &Msg> {
    fn __arg(&self) -> String {
        self.0.arg()
    }
}

#[doc(hidden)]
pub trait __ArgText {
    fn __arg(&self) -> String;
}

impl<T: fmt::Display + ?Sized> __ArgText for &__Arg<'_, T> {
    fn __arg(&self) -> String {
        format!("{}", self.0)
    }
}

#[doc(hidden)]
pub fn __args<const N: usize>(a: [String; N]) -> Vec<String> {
    Vec::from(a)
}
