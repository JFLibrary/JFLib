// pagescript -- the page scripts of the Webbrowser (B3): <script type="text/jfs"> in a page from
// the simulated network runs in apps/jfs/crates/script's VM with a host of its own, on the document the view
// shows (md_dom.rs through MdViewState::doc_mut); the Console a tab of the developer view shows
// (devtools.rs, B4)
//
//   PageScript::of(&view, html, url, seed) -> Option<Box<PageScript>>   the page's scripts in
//        document order, None without any (a page without scripts costs nothing); nothing runs yet
//   pump(&mut view, now, &mut seq)         what is due at virtual time `now`, up to FRAME_FUEL: the
//        next scripts, answers of fetch, timers (Browser: every pass while the document is shown)
//   events(&mut view, MdEv, now, &mut seq) -> (link, submit)   what the view reported goes to the
//        handlers; what comes back is what they left of the link's and the form's default action
//   answer(id, Ok(resp) | Err(why))        the outcome of a request of effects().fetch
//   set_location(url)                      the entry shown (a fragment link is an entry of its own
//        over the same document)
//   take_effects() -> Effects              what the browser does for the page: requests (WebReq,
//        the ids from `seq`), a navigation, a reload, the document changed
//   wake(now, running) -> Option<ms>       when a frame is needed for what waits; running: the
//        virtual clock moves (speed per mille)
//   console() / console_mut() / clear_console()   what the page printed, logged and got wrong
//   console_pane(ui, id, &console, &mut ConSeen)   the console's part of the developer view
//   jar_in(&mut jar, clock) / jar_out(&mut jar)    the browser lends its cookies around a pump or
//        events call (document.cookie; clock: the device clock's seconds)
//
// One VM per document (Doc::page): it comes and goes with the document, as the fields' values and
// the place do, so Back and Forward find a page as it was; it runs nothing while it is not shown
// (timers that fall due wait: one fires once when the page comes back, timer.every goes on from
// then, answers wait in order); Reload makes a new document and a new VM.  It is the window's
// state: no undo step, no project change, nothing of it in the engine but its requests.
//
// The globals (nothing else: no file of the device or of the real machine, no import, no net,
// disk or http of the script services; time and chance come from the simulation):
//   print(..), log(..), log.debug|info|warn|error(..)   into the console
//   now()                virtual time in ms (float); rand() / rand(n) / rand(a, b) as for services:
//                        a stream from the project's seed, the device, the address and the request
//   timer.after(ms, f) / timer.every(ms, f) -> id, timer.cancel(id)   on the simulation's clock
//   fetch(url | {url, method, headers, body}, fn(err, resp))   over the device's HTTP client (the
//                        address resolved against the page): resp {status, reason, headers, body,
//                        url, redirects}; err: why no answer came, as text (an answer of 404 is an
//                        answer); any http address (no same-origin rule in this simulator)
//   location             {href, scheme, host, hostname, port, path, query, fragment}, assign(url)
//                        (or location.href = url), reload(): after the callback, a fragment scrolls
//   document             by_id(id), find(sel), find_all(sel), create(tag), create_text(s), body(),
//                        title(), set_title(s), on(type, f), off(type, f); cookie: a text, the cookies
//                        of this address without the HttpOnly ones, as of the start of this call;
//                        document.cookie = "a=1; Path=/" stores one when the call ends (jfs has no
//                        setters: location.href does the same), set_cookie(s) at once, and the text
//                        is new after it.  What the jar refuses is a line of the console
//   an element           {kind, node, tag}: find(sel), find_all(sel), matches(sel), parent(),
//                        children() (elements), next(), prev(), text(), set_text(x), html(),
//                        set_html(s), attr(name), set_attr(name, x), remove_attr(name),
//                        has_class(c), add_class(c), remove_class(c), toggle_class(c) -> on,
//                        append(n), insert(n, before), remove() (kept: it may go in again),
//                        value(), set_value(x), checked(), set_checked(b) (form fields), on(type,
//                        f), off(type, f), same(other).  The same element is the same map (==)
//                        for the last ELEMS_KEPT elements a script touched; same() holds always
// Script elements that come later (set_html, create) do not run.
//
// Events: "click" (MdEv::click: the element under the pointer, a link's with Enter too; buttons,
// check boxes and radios), "input" (a text changed, a list chosen, a box toggled), "submit" (the
// form).  ev {kind, type, target, current, data} (data: a submit's name / value pairs as a map);
// ev.prevent() stops a link's or a form's default action, ev.stop() the way up.  They bubble: the
// handlers of the target, of each parent, of the document, in the order they came; no capture
// phase and no other default action can be stopped -- what a small page needs: a handler on the
// element, or one on a list or on the document for all that is in it.  A button's click comes
// before its form's submit.  A form goes with the values and the action as its handlers left them.
//
// Limits: the VM runs on the UI thread, so a page must not be able to hang it.  Each script, event
// handler, timer and answer is an invocation with CALL_FUEL of its own: at its end it stops with an
// error in the console, the page stays usable and the next one runs; a frame runs at most
// FRAME_FUEL (an invocation counts at least CALL_MIN), the rest waits for the next one.  Heap and
// call depth: LIMITS.  The document: PAGE_NODES, PAGE_BYTES, PAGE_DEPTH (the document is no part of
// the VM's heap); timers, handlers and open requests are capped too.  An operation costs fuel by
// its size.  Errors name their source (the page's address or the src) with line and column: an
// inline script keeps the lines and columns of the page (blanks before it), compile errors too.
//
// The scripts share their top-level names (Vm::add_script) and run in document order; a src script
// is fetched first (one at a time) and the next ones wait for it, a failed fetch is a console error
// and the next one runs; only type="text/jfs" runs.
//
// The console: lines with a kind, at most CON_LINES, each cut at CON_LINE bytes (the oldest go).
// print and log share the VM's output, so they stay in order: a line of log.warn / log.error starts
// with a mark that goes when the output comes to the console.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use base::rng::Pcg32;
use jfn_sim::{Ns, WebReq, WebResp, MS};
use netpkt::cookie::{Jar, Origin, Refused};
use netpkt::http::Headers;
use netpkt::uri::Uri;
use script::{num, Cx, Error, Fail, Host, Limits, Pin, Status, Value, Vm};
use ui::*;

use crate::app::tr::{t, tf};

/// the names a page's scripts have besides the built-ins and the standard library
const GLOBALS: &[&str] = &["log", "now", "rand", "timer", "fetch", "document", "location"];
/// what a script may read as a value (`let f = log`)
const FUNCS: &[&str] = &["log", "log.debug", "log.info", "log.warn", "log.error", "now", "rand", "timer.after", "timer.every", "timer.cancel", "fetch"];
/// fuel of one script, handler, timer or answer
pub(crate) const CALL_FUEL: u64 = 2_000_000;
/// fuel a frame runs at most, and what an invocation counts at least
const FRAME_FUEL: u64 = 4_000_000;
const CALL_MIN: u64 = 2_000;
const LIMITS: Limits = Limits { heap: 8 << 20, stack: 20_000, depth: 200, fuel: CALL_FUEL, output: 64 << 10 };
/// the document at most: nodes (out of it too), bytes of names, texts and attributes, depth
pub(crate) const PAGE_NODES: usize = 200_000;
const PAGE_BYTES: usize = 64 << 20;
const PAGE_DEPTH: usize = 1000;
/// how much a page writes between two looks at the document's size
const GROW_LOOK: usize = 256 << 10;
const TIMERS_MAX: usize = 4096;
const HANDLERS_MAX: usize = 4096;
const FETCHES_MAX: usize = 256;
/// the elements a script touched last keep their map
const ELEMS_KEPT: usize = 4096;
/// the shortest period of timer.every
const EVERY_MIN: Ns = MS;
/// bytes of a src script at most
const SRC_MAX: usize = 1 << 20;
const CON_LINES: usize = 1000;
const CON_LINE: usize = 2048;
const CON_SHOWN: usize = 200;
// the marks of log lines in the VM's output
const MARK_WARN: char = '\u{E000}';
const MARK_ERROR: char = '\u{E001}';

// the events
const ON_CLICK: u8 = 0;
const ON_INPUT: u8 = 1;
const ON_SUBMIT: u8 = 2;
const ON_NAMES: [&str; 3] = ["click", "input", "submit"];
// the node of the document's own handlers
const AT_DOCUMENT: u64 = 0;

// ---------------------------------------------------------------- the console

/// the kind of a console line
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConKind {
    Text,
    Warn,
    Error,
}

/// a line of the console
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConLine {
    pub kind: ConKind,
    pub text: String,
}

/// what a page printed, logged and got wrong
#[derive(Default)]
pub(crate) struct Console {
    lines: VecDeque<ConLine>,
    /// lines so far, errors so far (those gone too)
    total: u64,
    errors: u64,
}

impl Console {
    pub(crate) fn push(&mut self, kind: ConKind, text: &str) {
        let mut text = String::from(text);
        if text.len() > CON_LINE {
            let mut cut = CON_LINE;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('\u{2026}');
        }
        if self.lines.len() >= CON_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(ConLine { kind, text });
        self.total += 1;
        if kind == ConKind::Error {
            self.errors += 1;
        }
    }

    pub(crate) fn lines(&self) -> impl Iterator<Item = &ConLine> {
        self.lines.iter()
    }

    pub(crate) fn errors(&self) -> u64 {
        self.errors
    }

    pub(crate) fn clear(&mut self) {
        self.lines.clear();
        self.errors = 0;
        self.total += 1;
    }

    /// the lines as text, one per line (the checks)
    pub(crate) fn text(&self) -> String {
        let v: Vec<&str> = self.lines.iter().map(|l| l.text.as_str()).collect();
        v.join("\n")
    }
}

/// what the console's pane remembers: the lines it showed; new ones scroll to the end, once the
/// pane has measured them; a pane at its end stays there when the scale changes (the place is in
/// pixels and the lines take another number of them)
#[derive(Default)]
pub(crate) struct ConSeen {
    total: u64,
    /// a pass that goes to the end is due (the pane measures the lines first)
    scroll: u8,
    /// it was at the end in the last pass that had settled
    at_end: bool,
    /// the scale of the last pass
    dpi: i32,
}

/// the console's pane: `id` the scroll view, + 1 Leeren -> Leeren was pressed
pub(crate) fn console_pane(ui: &mut Ui, id: u32, c: &Console, seen: &mut ConSeen) -> bool {
    let mut clear = false;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.spacer(-1);
    ui.next_flags(ST_COMPACT | ST_SUBTLE);
    if ui.button(id + 1, t("Leeren")) & EV_ACT != 0 {
        clear = true;
    }
    ui.box_end();
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V);
    ui.box_begin(0, BOX_V, 2);
    if c.lines.is_empty() {
        ui.text(TS_CAPTION | TX_SECONDARY, t("Noch keine Ausgabe."));
    }
    let skip = c.lines.len().saturating_sub(CON_SHOWN);
    for l in c.lines.iter().skip(skip) {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H, 4);
        match l.kind {
            ConKind::Error => {
                ui.icon(IC_ERROR, 14);
            }
            ConKind::Warn => {
                ui.icon(IC_WARNING, 14);
            }
            ConKind::Text => {}
        }
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &l.text);
        ui.box_end();
    }
    ui.box_end();
    ui.scroll_end();
    let view = ui.px2dip(ui.last_rect()[3]);
    ui.box_end();
    let (y, full, dpi) = (ui.scroll_pos(id).1, ui.scroll_extent(id).1, ui.dpi());
    if seen.total != c.total {
        seen.total = c.total;
        seen.scroll = 1;
        seen.at_end = true;
        ui.repass();
    } else if seen.dpi != dpi && seen.at_end {
        // another scale: at the end stays at the end, once the lines are measured again
        seen.scroll = 1;
        ui.repass();
    } else if seen.scroll > 0 {
        seen.scroll -= 1;
        // (far enough for CON_SHOWN lines; more would overflow in pixels at a high scale)
        ui.scroll_to(id, -1, 1 << 20);
    } else {
        // (the user scrolls: wherever that ends)
        seen.at_end = y + view + 4 >= full;
    }
    seen.dpi = dpi;
    clear
}

// ---------------------------------------------------------------- the state

/// what the browser does for the page after a call
#[derive(Default)]
pub(crate) struct Effects {
    /// requests to send (Action::WebFetch), their ids from `seq`
    pub fetch: Vec<WebReq>,
    /// location.assign: a target, resolved against the page by the browser
    pub nav: Option<String>,
    pub reload: bool,
    /// the document changed (its title may have)
    pub changed: bool,
}

// a script of the page, in document order
enum Item {
    /// the text with blanks before it: the lines and columns of the page
    Inline(String),
    Src(String),
}

struct Tmr {
    at: Ns,
    period: Option<Ns>,
    pin: Pin,
}

// an event on its way
#[derive(Default)]
struct Event {
    serial: i64,
    prevented: bool,
    stopped: bool,
}

// what the host keeps
struct Shared {
    now: Ns,
    /// the browser's numbers for requests, while a call runs
    seq: u64,
    /// the timers' numbers
    next: u64,
    timers: BTreeMap<u64, Tmr>,
    /// by (node, event): the handlers in the order they came
    handlers: BTreeMap<(u64, u8), Vec<Pin>>,
    fetches: BTreeMap<u64, Pin>,
    /// the page's address as a script reads it, the base of what it fetches
    url: String,
    rng: Pcg32,
    console: Console,
    fx: Effects,
    /// elements with their maps; the newest numbers go last
    elems: BTreeMap<u64, Pin>,
    event: Event,
    grown: usize,
    /// nodes of the document at the last look: what a query costs
    nodes: usize,
    document: Value,
    /// the browser's cookies while a call runs, the device clock's seconds, the text document.cookie
    /// holds (None: not written yet)
    jar: Arc<Jar>,
    clock: i64,
    cookie_seen: Option<String>,
}

impl Shared {
    // a cookie from the page -> what to say when the jar refuses it
    fn store_cookie(&mut self, text: &str) -> Option<String> {
        let u = Uri::parse(&self.url);
        let host = String::from(u.host().unwrap_or(""));
        let secure = u.scheme.as_deref() == Some("https");
        let o = Origin { host: &host, path: if u.path.is_empty() { "/" } else { &u.path }, secure, http: false };
        if host.is_empty() {
            return None;
        }
        let clock = self.clock;
        let Err(e) = Arc::make_mut(&mut self.jar).store(text, o, clock) else { return None };
        let name = text.split(['=', ';']).next().unwrap_or("").trim();
        Some(tf("Cookie {} nicht gespeichert: {}", &[name, refused_text(e)]))
    }

    // what document.cookie reads now
    fn cookie_text(&self) -> String {
        let u = Uri::parse(&self.url);
        let Some(host) = u.host() else { return String::new() };
        self.jar.script_text(host, if u.path.is_empty() { "/" } else { &u.path }, u.scheme.as_deref() == Some("https"), self.clock)
    }
}

/// why the jar refuses a cookie, in words
pub(crate) fn refused_text(e: Refused) -> &'static str {
    match e {
        Refused::Syntax => t("kein Name"),
        Refused::TooLarge => t("zu groß"),
        Refused::Domain => t("die Domain passt nicht zur Adresse"),
        Refused::HttpOnly => t("HttpOnly dürfen Skripte nicht setzen oder überschreiben"),
        Refused::Secure => t("Secure geht nur über https"),
        Refused::SameSiteNone => t("SameSite=None braucht Secure"),
        _ => t("abgelehnt"),
    }
}

/// the scripts of a document (see the module header)
pub(crate) struct PageScript {
    vm: Vm,
    s: Shared,
    location: Value,
    items: VecDeque<Item>,
    /// the src script on its way: its request, its address
    loading: Option<(u64, String)>,
    /// its outcome
    loaded: Option<(String, Result<WebResp, String>)>,
    /// answers of fetch in the order they came
    ready: VecDeque<(u64, Result<WebResp, String>)>,
}

// ---------------------------------------------------------------- small parts

fn ms_of(v: Value) -> Option<Ns> {
    match v {
        Value::Int(i) if i >= 0 => Some((i as u64).saturating_mul(MS)),
        Value::Float(f) if !num::is_nan(f) && num::le(0.0, f) => {
            let ns = num::floor(num::add(num::mul(f, 1_000_000.0), 0.5));
            Some(num::to_i64(ns).map_or(u64::MAX, |n| n as u64))
        }
        _ => None,
    }
}

fn ms_float(t: Ns) -> f64 {
    num::div(num::from_i64(t.min(i64::MAX as u64) as i64), 1_000_000.0)
}

fn event_of(name: &str) -> Option<u8> {
    ON_NAMES.iter().position(|e| *e == name).map(|i| i as u8)
}

// a script's text with blanks before it: its lines and columns are those of the page; `from`: where
// the search starts in the page, then behind the text
fn lined_up(html: &str, from: &mut usize, text: &str) -> String {
    let Some(i) = html.get(*from..).and_then(|h| h.find(text)) else { return String::from(text) };
    let at = *from + i;
    *from = at + text.len();
    let before = &html[..at];
    let line = before.matches('\n').count();
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count());
    format!("{}{}{}", "\n".repeat(line), " ".repeat(col), text)
}

// an error of the VM as console lines
fn error_lines(e: &Error) -> Vec<String> {
    let mut v = Vec::new();
    if e.line > 0 {
        v.push(format!("{}:{}:{}: {}", e.file, e.line, e.col, e.msg));
    } else {
        v.push(e.msg.clone());
    }
    for tr in e.trace.iter().skip(1).take(4) {
        if tr.func.is_empty() {
            v.push(format!("  ({}:{}:{})", tr.file, tr.line, tr.col));
        } else {
            v.push(format!("  {} ({}:{}:{})", tr.func, tr.file, tr.line, tr.col));
        }
    }
    v
}

// the map of an element or text outside a run (an event's target): the one a script has, if any
fn node_map(vm: &mut Vm, s: &mut Shared, doc: &MdDoc, n: MdNode) -> Result<Value, Error> {
    if let Some(p) = s.elems.get(&n.raw()) {
        return Ok(vm.pinned(*p));
    }
    let m = vm.new_map()?;
    let (k, v) = (vm.new_str("kind")?, vm.new_str(if doc.is_text(n) { "text" } else { "element" })?);
    vm.map_set(m, k, v)?;
    let k = vm.new_str("node")?;
    vm.map_set(m, k, Value::Int(n.raw() as i64))?;
    let (k, v) = (vm.new_str("tag")?, vm.new_str(doc.tag(n))?);
    vm.map_set(m, k, v)?;
    if s.elems.len() >= ELEMS_KEPT {
        for (_, p) in core::mem::take(&mut s.elems) {
            vm.unpin(p);
        }
    }
    let p = vm.pin(m);
    s.elems.insert(n.raw(), p);
    Ok(m)
}

fn set_str(vm: &mut Vm, m: Value, key: &str, val: &str) -> Result<(), Error> {
    let (k, v) = (vm.new_str(key)?, vm.new_str(val)?);
    vm.map_set(m, k, v)
}

fn headers_of(vm: &mut Vm, h: &Headers) -> Result<Value, Error> {
    let m = vm.new_map()?;
    for (k, v) in &h.0 {
        set_str(vm, m, &k.to_ascii_lowercase(), v)?;
    }
    Ok(m)
}

// the parts of an address into location's map
fn fill_location(vm: &mut Vm, m: Value, url: &str) -> Result<(), Error> {
    let u = Uri::parse(url);
    set_str(vm, m, "kind", "location")?;
    set_str(vm, m, "href", url)?;
    set_str(vm, m, "scheme", u.scheme.as_deref().unwrap_or(""))?;
    set_str(vm, m, "host", u.authority.as_deref().unwrap_or(""))?;
    set_str(vm, m, "hostname", u.host().unwrap_or(""))?;
    let k = vm.new_str("port")?;
    vm.map_set(m, k, u.port().map_or(Value::Nil, |p| Value::Int(p as i64)))?;
    set_str(vm, m, "path", &u.path)?;
    set_str(vm, m, "query", u.query.as_deref().unwrap_or(""))?;
    set_str(vm, m, "fragment", u.fragment.as_deref().unwrap_or(""))
}

// ---------------------------------------------------------------- the host

struct PageHost<'a> {
    view: &'a mut MdViewState,
    s: &'a mut Shared,
}

impl Host for PageHost<'_> {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let a = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        match name {
            "log" | "log.debug" | "log.info" => self.log(cx, None, args),
            "log.warn" => self.log(cx, Some(MARK_WARN), args),
            "log.error" => self.log(cx, Some(MARK_ERROR), args),
            "now" => Ok(Value::Float(ms_float(self.s.now))),
            "rand" => self.rand(cx, a(0), a(1)),
            "timer.after" | "timer.every" => self.timer(cx, name, a(0), a(1)),
            "timer.cancel" => self.cancel(cx, a(0)),
            "fetch" => self.fetch(cx, a(0), a(1)),
            _ => match name.strip_prefix("map.") {
                Some(m) => self.method(cx, m, args),
                None => Err(Fail::Unknown),
            },
        }
    }

    fn knows(&self, name: &str) -> bool {
        FUNCS.contains(&name)
    }

    fn load(&mut self, _: &mut Cx, _: &str) -> Result<String, String> {
        Err(String::from(t("Seitenskripte lesen keine Dateien: import geht hier nicht.")))
    }
}

impl PageHost<'_> {
    fn log(&mut self, cx: &mut Cx, mark: Option<char>, args: &[Value]) -> Result<Value, Fail> {
        let parts: Vec<String> = args.iter().map(|v| cx.display(*v)).collect();
        let mut text = parts.join(" ");
        cx.charge(text.len() as u64 / 64);
        text.push('\n');
        // (in the output, so print and log stay in order)
        match mark {
            Some(m) => {
                let marked: Vec<String> = text.lines().map(|l| format!("{}{}\n", m, l)).collect();
                cx.print(&marked.concat());
            }
            None => cx.print(&text),
        }
        Ok(Value::Nil)
    }

    fn rand(&mut self, cx: &mut Cx, a: Value, b: Value) -> Result<Value, Fail> {
        match (a, b) {
            (Value::Nil, Value::Nil) => {
                let x = self.s.rng.next_u64() >> 11;
                Ok(Value::Float(num::mul(num::from_i64(x as i64), 1.0 / 9_007_199_254_740_992.0)))
            }
            (Value::Int(n), Value::Nil) if n >= 1 => Ok(Value::Int(self.s.rng.range(0, n - 1))),
            (Value::Int(lo), Value::Int(hi)) if lo <= hi => Ok(Value::Int(self.s.rng.range(lo, hi))),
            _ => Err(cx.error(t("rand() gibt eine Zahl in [0, 1), rand(n) eine ganze Zahl von 0 bis n - 1, rand(a, b) eine von a bis b"))),
        }
    }

    fn timer(&mut self, cx: &mut Cx, name: &str, ms: Value, f: Value) -> Result<Value, Fail> {
        let Some(d) = ms_of(ms) else {
            return Err(cx.error(&tf("{}: die Zeit muss eine Zahl ab 0 sein (Millisekunden)", &[name])));
        };
        if !matches!(f, Value::Func(_)) {
            return Err(cx.error(&tf("{}: hier muss eine Funktion stehen", &[name])));
        }
        if self.s.timers.len() >= TIMERS_MAX {
            return Err(cx.error(t("Zu viele Zeitgeber auf dieser Seite.")));
        }
        let period = (name == "timer.every").then(|| d.max(EVERY_MIN));
        let at = self.s.now.saturating_add(period.unwrap_or(d));
        self.s.next += 1;
        let id = self.s.next;
        let pin = cx.pin(f);
        self.s.timers.insert(id, Tmr { at, period, pin });
        Ok(Value::Int(id as i64))
    }

    fn cancel(&mut self, cx: &mut Cx, v: Value) -> Result<Value, Fail> {
        let Value::Int(id) = v else { return Ok(Value::Bool(false)) };
        match self.s.timers.remove(&(id as u64)) {
            Some(tm) => {
                cx.unpin(tm.pin);
                Ok(Value::Bool(true))
            }
            None => Ok(Value::Bool(false)),
        }
    }

    fn fetch(&mut self, cx: &mut Cx, what: Value, f: Value) -> Result<Value, Fail> {
        if !matches!(f, Value::Func(_)) {
            return Err(cx.error(&tf("{}: hier muss eine Funktion stehen", &["fetch"])));
        }
        let (url, opts) = match what {
            Value::Map(_) => (cx.map_get_str(what, "url").unwrap_or(Value::Nil), what),
            v => (v, Value::Nil),
        };
        let Some(text) = cx.str(url).map(String::from) else {
            return Err(cx.error(t("fetch: die URL fehlt")));
        };
        if self.s.fetches.len() >= FETCHES_MAX {
            return Err(cx.error(t("Zu viele offene Anfragen auf dieser Seite.")));
        }
        let mut headers = Headers::new();
        let (mut method, mut body) = (String::from("GET"), Vec::new());
        if matches!(opts, Value::Map(_)) {
            if let Some(m) = cx.map_get_str(opts, "method").and_then(|v| cx.str(v)) {
                method = m.to_ascii_uppercase();
            }
            if let Some(b) = cx.map_get_str(opts, "body") {
                body = match cx.str(b) {
                    Some(s) => Vec::from(s.as_bytes()),
                    None => cx.bytes(b).map(Vec::from).unwrap_or_default(),
                };
            }
            if let Some(h) = cx.map_get_str(opts, "headers") {
                for (k, v) in cx.map_entries(h) {
                    let (k, v) = (cx.display(k), cx.display(v));
                    headers.add(&k, &v);
                }
            }
        }
        cx.charge(body.len() as u64 / 16);
        self.s.seq += 1;
        let id = self.s.seq;
        let pin = cx.pin(f);
        self.s.fetches.insert(id, pin);
        let u = Uri::parse(&self.s.url).join(text.trim()).encoded();
        self.s.fx.fetch.push(WebReq { id, method, url: u.text(), headers, body, ..WebReq::default() });
        Ok(Value::Nil)
    }

    // obj.m(..) on a map: an element, the document, an event, location; any other map has no m
    fn method(&mut self, cx: &mut Cx, m: &str, args: &[Value]) -> Result<Value, Fail> {
        let this = args.first().copied().unwrap_or(Value::Nil);
        let kind = cx.map_get_str(this, "kind").and_then(|v| cx.str(v)).map_or(0, |k| match k {
            "element" | "text" => 1,
            "document" => 2,
            "event" => 3,
            "location" => 4,
            _ => 0,
        });
        let a = |i: usize| args.get(i + 1).copied().unwrap_or(Value::Nil);
        cx.charge(16);
        match kind {
            1 => {
                let n = self.node_of(cx, this);
                self.element(cx, m, n, a(0), a(1))
            }
            2 => self.document(cx, m, a(0), a(1)),
            3 => {
                let live = matches!(cx.map_get_str(this, "serial"), Some(Value::Int(s)) if s == self.s.event.serial);
                match m {
                    "prevent" => self.s.event.prevented |= live,
                    "stop" => self.s.event.stopped |= live,
                    _ => return Err(Fail::Unknown),
                }
                Ok(Value::Nil)
            }
            4 => match m {
                "assign" => {
                    let Some(u) = cx.str(a(0)).map(String::from) else {
                        return Err(cx.error(t("location.assign: die Adresse fehlt")));
                    };
                    self.s.fx.nav = Some(u);
                    Ok(Value::Nil)
                }
                "reload" => {
                    self.s.fx.reload = true;
                    Ok(Value::Nil)
                }
                _ => Err(Fail::Unknown),
            },
            _ => Err(Fail::Unknown),
        }
    }

    // the node of a map of ours (a stale one: NONE, what the document answers with nothing)
    fn node_of(&self, cx: &Cx, v: Value) -> MdNode {
        match cx.map_get_str(v, "node") {
            Some(Value::Int(i)) => {
                let n = MdNode::from_raw(i as u64);
                if self.view.doc().alive(n) { n } else { MdNode::NONE }
            }
            _ => MdNode::NONE,
        }
    }

    // the map of a node (the same one again while it is kept: == holds)
    fn el(&mut self, cx: &mut Cx, n: MdNode) -> Result<Value, Fail> {
        if n.is_none() {
            return Ok(Value::Nil);
        }
        if let Some(p) = self.s.elems.get(&n.raw()) {
            return Ok(cx.pinned(*p));
        }
        let doc = self.view.doc();
        let m = cx.new_map()?;
        let (k, v) = (cx.new_str("kind")?, cx.new_str(if doc.is_text(n) { "text" } else { "element" })?);
        cx.map_set(m, k, v)?;
        let k = cx.new_str("node")?;
        cx.map_set(m, k, Value::Int(n.raw() as i64))?;
        let (k, v) = (cx.new_str("tag")?, cx.new_str(doc.tag(n))?);
        cx.map_set(m, k, v)?;
        cx.charge(32);
        if self.s.elems.len() >= ELEMS_KEPT {
            for (_, p) in core::mem::take(&mut self.s.elems) {
                cx.unpin(p);
            }
        }
        let p = cx.pin(m);
        self.s.elems.insert(n.raw(), p);
        Ok(m)
    }

    fn els(&mut self, cx: &mut Cx, nodes: Vec<MdNode>) -> Result<Value, Fail> {
        let l = cx.new_list(Vec::with_capacity(nodes.len()))?;
        for n in nodes {
            let m = self.el(cx, n)?;
            cx.list_push(l, m)?;
        }
        Ok(l)
    }

    // a selector's nodes: a query costs by the document's size
    fn query(&mut self, cx: &mut Cx, name: &str, at: Option<MdNode>, sel: Value) -> Result<Vec<MdNode>, Fail> {
        let Some(s) = cx.str(sel).map(String::from) else {
            return Err(cx.error(&tf("{}: hier muss ein Text stehen", &[name])));
        };
        cx.spend(self.s.nodes as u64 * 2 + 64)?;
        let d = self.view.doc();
        Ok(match at {
            Some(n) => d.select_in(n, &s),
            None => d.select(&s),
        })
    }

    // room for what a call adds to the document: a look at its size now and then
    fn room(&mut self, cx: &mut Cx, nodes: usize, bytes: usize) -> Result<(), Fail> {
        self.s.grown += bytes + nodes * 64;
        cx.charge(nodes as u64 * 64 + bytes as u64 / 16);
        if self.s.grown >= GROW_LOOK {
            self.s.grown = 0;
            let (n, b) = self.view.doc().dom_size();
            self.s.nodes = n;
            cx.charge(n as u64);
            if n > PAGE_NODES || b > PAGE_BYTES {
                return Err(cx.error(t("Das Dokument dieser Seite ist zu groß geworden.")));
            }
        }
        Ok(())
    }

    fn on(&mut self, cx: &mut Cx, at: u64, ty: Value, f: Value) -> Result<Value, Fail> {
        let Some(ev) = cx.str(ty).and_then(event_of) else {
            return Err(cx.error(&tf("{}: das Ereignis muss click, input oder submit sein", &["on"])));
        };
        if !matches!(f, Value::Func(_)) {
            return Err(cx.error(&tf("{}: hier muss eine Funktion stehen", &["on"])));
        }
        if self.s.handlers.values().map(|v| v.len()).sum::<usize>() >= HANDLERS_MAX {
            return Err(cx.error(t("Zu viele Ereignis-Funktionen auf dieser Seite.")));
        }
        let p = cx.pin(f);
        self.s.handlers.entry((at, ev)).or_default().push(p);
        Ok(Value::Nil)
    }

    // off(type, f): that handler; off(type): all of them
    fn off(&mut self, cx: &mut Cx, at: u64, ty: Value, f: Value) -> Result<Value, Fail> {
        let Some(ev) = cx.str(ty).and_then(event_of) else {
            return Err(cx.error(&tf("{}: das Ereignis muss click, input oder submit sein", &["off"])));
        };
        if let Some(v) = self.s.handlers.get_mut(&(at, ev)) {
            let mut keep = Vec::with_capacity(v.len());
            for p in v.drain(..) {
                if f == Value::Nil || cx.equal(cx.pinned(p), f) {
                    cx.unpin(p);
                } else {
                    keep.push(p);
                }
            }
            *v = keep;
            if v.is_empty() {
                self.s.handlers.remove(&(at, ev));
            }
        }
        Ok(Value::Nil)
    }

    fn document(&mut self, cx: &mut Cx, m: &str, a: Value, b: Value) -> Result<Value, Fail> {
        match m {
            "by_id" => {
                let Some(id) = cx.str(a).map(String::from) else {
                    return Err(cx.error(&tf("{}: hier muss ein Text stehen", &["document.by_id"])));
                };
                let n = self.view.doc().by_id(&id).unwrap_or_default();
                self.el(cx, n)
            }
            "find" => {
                let v = self.query(cx, "document.find", None, a)?;
                self.el(cx, v.first().copied().unwrap_or_default())
            }
            "find_all" => {
                let v = self.query(cx, "document.find_all", None, a)?;
                self.els(cx, v)
            }
            "create" => {
                let Some(tag) = cx.str(a).map(String::from) else {
                    return Err(cx.error(&tf("{}: hier muss ein Text stehen", &["document.create"])));
                };
                self.room(cx, 1, tag.len())?;
                let n = self.view.doc_mut().create_element(&tag);
                self.el(cx, n)
            }
            "create_text" => {
                let s = cx.display(a);
                self.room(cx, 1, s.len())?;
                let n = self.view.doc_mut().create_text(&s);
                self.el(cx, n)
            }
            "body" => {
                let d = self.view.doc();
                let n = d.select_first("body").unwrap_or(d.root());
                self.el(cx, n)
            }
            "title" => {
                let d = self.view.doc();
                let s = d.select_first("title").map_or(String::new(), |n| d.text_of(n));
                cx.new_str(&s)
            }
            "set_title" => {
                let s = cx.display(a);
                self.room(cx, 1, s.len())?;
                let d = self.view.doc_mut();
                let n = match d.select_first("title") {
                    Some(n) => n,
                    None => {
                        let n = d.create_element("title");
                        let root = d.root();
                        d.insert(root, None, n);
                        n
                    }
                };
                d.set_text(n, &s);
                self.s.fx.changed = true;
                Ok(Value::Nil)
            }
            "set_cookie" => {
                let Some(x) = cx.str(a).map(String::from) else {
                    return Err(cx.error(&tf("{}: hier muss ein Text stehen", &["document.set_cookie"])));
                };
                cx.charge(x.len() as u64 / 16 + 64);
                let document = self.s.document;
                // (what this call assigned before comes first; the words in the output's order)
                let assigned = cx.map_get_str(document, "cookie").and_then(|v| cx.str(v)).map(String::from);
                let mut said = None;
                if let Some(w) = assigned.filter(|w| self.s.cookie_seen.as_deref() != Some(w.as_str())) {
                    said = self.s.store_cookie(&w);
                }
                for l in said.into_iter().chain(self.s.store_cookie(&x)) {
                    cx.print(&format!("{}{}\n", MARK_WARN, l));
                }
                let text = self.s.cookie_text();
                let (k, v) = (cx.new_str("cookie")?, cx.new_str(&text)?);
                cx.map_set(document, k, v)?;
                self.s.cookie_seen = Some(text);
                Ok(Value::Nil)
            }
            "on" => self.on(cx, AT_DOCUMENT, a, b),
            "off" => self.off(cx, AT_DOCUMENT, a, b),
            _ => Err(Fail::Unknown),
        }
    }

    // the field of an element, with the document's changes taken
    fn field(&mut self, cx: &mut Cx, n: MdNode) -> Option<usize> {
        if self.view.doc().changed() {
            cx.charge(self.s.nodes as u64 * 4 + 256);
        }
        self.view.update();
        self.view.doc().field_of(n)
    }

    fn element(&mut self, cx: &mut Cx, m: &str, n: MdNode, a: Value, b: Value) -> Result<Value, Fail> {
        // (a node that is gone answers with nothing)
        let d = self.view.doc();
        match m {
            "same" => Ok(Value::Bool(!n.is_none() && self.node_of(cx, a) == n)),
            "text" => {
                let s = d.text_of(n);
                cx.charge(s.len() as u64 / 16);
                cx.new_str(&s)
            }
            "html" => {
                let s = d.inner_html(n);
                cx.charge(s.len() as u64 / 16);
                cx.new_str(&s)
            }
            "attr" => {
                let Some(name) = cx.str(a) else {
                    return Err(cx.error(&tf("{}: hier muss ein Text stehen", &["attr"])));
                };
                match d.attr(n, name).map(String::from) {
                    Some(v) => cx.new_str(&v),
                    None => Ok(Value::Nil),
                }
            }
            "has_class" => Ok(Value::Bool(cx.str(a).is_some_and(|c| d.has_class(n, c)))),
            "matches" => {
                let Some(s) = cx.str(a) else {
                    return Err(cx.error(&tf("{}: hier muss ein Text stehen", &["matches"])));
                };
                Ok(Value::Bool(d.matches(n, s)))
            }
            "parent" => {
                let p = d.parent(n).filter(|p| *p != d.root()).unwrap_or_default();
                self.el(cx, p)
            }
            "next" | "prev" => {
                let mut x = if m == "next" { d.next_sibling(n) } else { d.prev_sibling(n) };
                while let Some(y) = x.filter(|y| d.is_text(*y)) {
                    x = if m == "next" { d.next_sibling(y) } else { d.prev_sibling(y) };
                }
                self.el(cx, x.unwrap_or_default())
            }
            "children" => {
                let v: Vec<MdNode> = d.children(n).into_iter().filter(|c| !d.is_text(*c)).collect();
                cx.charge(v.len() as u64);
                self.els(cx, v)
            }
            "find" => {
                let v = self.query(cx, "find", Some(n), a)?;
                self.el(cx, v.first().copied().unwrap_or_default())
            }
            "find_all" => {
                let v = self.query(cx, "find_all", Some(n), a)?;
                self.els(cx, v)
            }
            "set_text" => {
                let s = cx.display(a);
                self.room(cx, 1, s.len())?;
                self.view.doc_mut().set_text(n, &s);
                self.s.fx.changed = true;
                Ok(Value::Nil)
            }
            "set_html" => {
                let s = cx.display(a);
                self.room(cx, s.len() / 8, s.len())?;
                self.view.doc_mut().set_inner_html(n, &s);
                self.s.fx.changed = true;
                Ok(Value::Nil)
            }
            "set_attr" | "remove_attr" | "add_class" | "remove_class" | "toggle_class" => {
                let Some(x) = cx.str(a).map(String::from) else {
                    return Err(cx.error(&tf("{}: hier muss ein Text stehen", &[m])));
                };
                let v = if m == "set_attr" { cx.display(b) } else { String::new() };
                self.room(cx, 0, x.len() + v.len())?;
                let d = self.view.doc_mut();
                let r = match m {
                    "set_attr" => d.set_attr(n, &x, &v),
                    "remove_attr" => d.remove_attr(n, &x),
                    "add_class" => d.add_class(n, &x),
                    "remove_class" => d.remove_class(n, &x),
                    _ => d.toggle_class(n, &x),
                };
                self.s.fx.changed = true;
                Ok(Value::Bool(r))
            }
            "append" | "insert" => {
                let c = self.node_of(cx, a);
                let before = if m == "insert" && b != Value::Nil { Some(self.node_of(cx, b)) } else { None };
                let d = self.view.doc();
                let mut depth = 0;
                let mut x = d.parent(n);
                while let Some(y) = x {
                    depth += 1;
                    x = d.parent(y);
                }
                cx.charge(depth as u64 + 16);
                if depth >= PAGE_DEPTH {
                    return Err(cx.error(t("Das Dokument dieser Seite ist zu tief verschachtelt.")));
                }
                if !self.view.doc_mut().insert(n, before, c) {
                    return Err(cx.error(t("Das Element lässt sich hier nicht einfügen.")));
                }
                self.s.fx.changed = true;
                Ok(Value::Nil)
            }
            "remove" => {
                self.view.doc_mut().remove(n);
                self.s.fx.changed = true;
                Ok(Value::Nil)
            }
            "value" => {
                let Some(i) = self.field(cx, n) else {
                    return match self.view.doc().attr(n, "value").map(String::from) {
                        Some(v) => cx.new_str(&v),
                        None => Ok(Value::Nil),
                    };
                };
                let d = self.view.doc();
                let fd = &d.fields()[i];
                let v = match fd.kind {
                    MF_SELECT => fd.options.get(self.view.field_selected(i).max(0) as usize).map_or(String::new(), |o| o.value.clone()),
                    MF_CHECKBOX | MF_RADIO | MF_SUBMIT | MF_RESET | MF_BUTTON => fd.value.clone(),
                    _ => String::from(self.view.field_text(i)),
                };
                cx.new_str(&v)
            }
            "set_value" => {
                let s = cx.display(a);
                self.room(cx, 0, s.len())?;
                let Some(i) = self.field(cx, n) else { return Ok(Value::Nil) };
                let fd = &self.view.doc().fields()[i];
                match fd.kind {
                    MF_SELECT => {
                        if let Some(k) = fd.options.iter().position(|o| o.value == s) {
                            self.view.set_field_selected(i, k as i32);
                        }
                    }
                    MF_CHECKBOX | MF_RADIO | MF_SUBMIT | MF_RESET | MF_BUTTON => {
                        self.view.doc_mut().set_attr(n, "value", &s);
                    }
                    _ => self.view.set_field_text(i, &s),
                }
                self.s.fx.changed = true;
                Ok(Value::Nil)
            }
            "checked" => {
                let on = self.field(cx, n).is_some_and(|i| self.view.field_checked(i));
                Ok(Value::Bool(on))
            }
            "set_checked" => {
                if let Some(i) = self.field(cx, n) {
                    self.view.set_field_checked(i, a.truthy());
                    self.s.fx.changed = true;
                }
                Ok(Value::Nil)
            }
            "on" if !n.is_none() => self.on(cx, n.raw(), a, b),
            "off" if !n.is_none() => self.off(cx, n.raw(), a, b),
            "on" | "off" => Ok(Value::Nil),
            _ => Err(Fail::Unknown),
        }
    }
}

// ---------------------------------------------------------------- the page's life

impl PageScript {
    /// the scripts of a page (see the module header); seed: the project's, the device's and the
    /// request's numbers mixed
    pub(crate) fn of(view: &MdViewState, html: &str, url: &str, seed: u64) -> Option<Box<PageScript>> {
        let doc = view.doc();
        let mut items = VecDeque::new();
        let mut from = 0;
        for n in doc.select("script") {
            if !doc.attr(n, "type").is_some_and(|x| x.trim().eq_ignore_ascii_case("text/jfs")) {
                continue;
            }
            match doc.attr(n, "src").filter(|x| !x.trim().is_empty()) {
                Some(src) => items.push_back(Item::Src(Uri::parse(url).join(src.trim()).encoded().text())),
                None => {
                    let text = doc.text_of(n);
                    if !text.trim().is_empty() {
                        items.push_back(Item::Inline(lined_up(html, &mut from, &text)));
                    }
                }
            }
        }
        if items.is_empty() {
            return None;
        }
        let stub = script::compile_with("", "", GLOBALS).ok()?;
        let mut vm = Vm::new(&stub, LIMITS);
        let mut host = ();
        if !matches!(vm.run(&mut host, u64::MAX), Status::Done(_)) {
            return None;
        }
        let location = vm.new_map().ok()?;
        fill_location(&mut vm, location, url).ok()?;
        vm.set_global("location", location);
        let document = vm.new_map().ok()?;
        set_str(&mut vm, document, "kind", "document").ok()?;
        vm.set_global("document", document);
        let mut mix = 0xCBF2_9CE4_8422_2325u64;
        for b in url.bytes() {
            mix = (mix ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3);
        }
        let s = Shared {
            now: 0,
            seq: 0,
            next: 0,
            timers: BTreeMap::new(),
            handlers: BTreeMap::new(),
            fetches: BTreeMap::new(),
            url: String::from(url),
            rng: Pcg32::new(seed, mix),
            console: Console::default(),
            fx: Effects::default(),
            elems: BTreeMap::new(),
            event: Event::default(),
            grown: 0,
            nodes: doc.dom_size().0,
            document,
            jar: Arc::new(Jar::new()),
            clock: 0,
            cookie_seen: None,
        };
        Some(Box::new(PageScript { vm, s, location, items, loading: None, loaded: None, ready: VecDeque::new() }))
    }

    /// the browser's cookies for the next pump or events call
    pub(crate) fn jar_in(&mut self, jar: &mut Arc<Jar>, clock: i64) {
        core::mem::swap(&mut self.s.jar, jar);
        self.s.clock = clock;
    }

    /// and back, as the scripts left them
    pub(crate) fn jar_out(&mut self, jar: &mut Arc<Jar>) {
        core::mem::swap(&mut self.s.jar, jar);
    }

    // document.cookie as of now
    fn sync_cookie(&mut self) {
        let text = self.s.cookie_text();
        if self.s.cookie_seen.as_deref() != Some(text.as_str()) {
            if let Err(e) = set_str(&mut self.vm, self.s.document, "cookie", &text) {
                self.s.console.push(ConKind::Error, &e.msg);
            }
            self.s.cookie_seen = Some(text);
        }
    }

    // document.cookie = ...: what a script left there goes to the jar
    fn take_cookie(&mut self) {
        let written = self.vm.map_get_str(self.s.document, "cookie").and_then(|v| self.vm.str(v)).map(String::from);
        if let Some(w) = written {
            if self.s.cookie_seen.as_deref() != Some(w.as_str()) {
                if let Some(l) = self.s.store_cookie(&w) {
                    self.s.console.push(ConKind::Warn, &l);
                }
                self.s.cookie_seen = None;
            }
        } else {
            self.s.cookie_seen = None;
        }
        self.sync_cookie();
    }

    // what print and log wrote, into the console
    fn flush(&mut self) {
        let out = self.vm.take_output();
        for l in out.lines() {
            match l.chars().next() {
                Some(MARK_WARN) => self.s.console.push(ConKind::Warn, &l[MARK_WARN.len_utf8()..]),
                Some(MARK_ERROR) => self.s.console.push(ConKind::Error, &l[MARK_ERROR.len_utf8()..]),
                _ => self.s.console.push(ConKind::Text, l),
            }
        }
    }

    // the pending invocation to its end -> the fuel it counts
    fn finish(&mut self, view: &mut MdViewState) -> u64 {
        let mut host = PageHost { view, s: &mut self.s };
        match self.vm.run(&mut host, u64::MAX) {
            Status::Done(_) => {}
            Status::Error(e) => {
                self.flush();
                for l in error_lines(&e) {
                    self.s.console.push(ConKind::Error, &l);
                }
            }
            // (nothing here yields or pauses)
            _ => self.vm.cancel(),
        }
        self.flush();
        // location.href = ...
        if let Some(h) = self.vm.map_get_str(self.location, "href").and_then(|v| self.vm.str(v)) {
            if h != self.s.url {
                self.s.fx.nav = Some(String::from(h));
                let url = self.s.url.clone();
                self.write_location(&url);
            }
        }
        self.take_cookie();
        self.vm.call_fuel().max(CALL_MIN)
    }

    // a function value with these arguments -> the fuel it counts
    fn invoke(&mut self, view: &mut MdViewState, f: Value, args: &[Value]) -> u64 {
        self.sync_cookie();
        if let Err(e) = self.vm.call_value(f, args) {
            self.s.console.push(ConKind::Error, &e.msg);
            return CALL_MIN;
        }
        self.finish(view)
    }

    fn write_location(&mut self, url: &str) {
        if let Err(e) = fill_location(&mut self.vm, self.location, url) {
            self.s.console.push(ConKind::Error, &e.msg);
        }
    }

    /// the entry shown has this address
    pub(crate) fn set_location(&mut self, url: &str) {
        if self.s.url != url {
            self.s.url = String::from(url);
            self.write_location(url);
        }
    }

    // a script's source into the VM and to its end
    fn run_script(&mut self, view: &mut MdViewState, name: &str, src: &str) -> u64 {
        self.sync_cookie();
        match self.vm.add_script(name, src) {
            Err(ds) => {
                for d in ds.iter().take(5) {
                    let l = if d.line > 0 { format!("{}:{}:{}: {}", d.file, d.line, d.col, d.msg) } else { format!("{}: {}", d.file, d.msg) };
                    self.s.console.push(ConKind::Error, &l);
                }
                if ds.len() > 5 {
                    self.s.console.push(ConKind::Error, &tf("{}: und {} weitere Fehler", &[name, &(ds.len() - 5).to_string()]));
                }
                CALL_MIN
            }
            Ok(()) => self.finish(view),
        }
    }

    // the next scripts, as far as the budget goes and none is on its way
    fn run_items(&mut self, view: &mut MdViewState, budget: &mut u64) {
        while *budget > 0 {
            if let Some((url, r)) = self.loaded.take() {
                match r {
                    Ok(a) if a.status >= 400 => {
                        let l = tf("{}: das Skript kam nicht: {} {}", &[&url, &a.status.to_string(), &a.reason]);
                        self.s.console.push(ConKind::Error, &l);
                    }
                    Ok(a) if a.body.len() > SRC_MAX => {
                        let l = tf("{}: das Skript ist zu groß ({} Bytes)", &[&url, &a.body.len().to_string()]);
                        self.s.console.push(ConKind::Error, &l);
                    }
                    Ok(a) => {
                        let text = String::from_utf8_lossy(&a.body).into_owned();
                        let used = self.run_script(view, &url, text.strip_prefix('\u{FEFF}').unwrap_or(&text));
                        *budget = budget.saturating_sub(used);
                    }
                    Err(why) => {
                        let l = tf("{}: das Skript kam nicht: {}", &[&url, &why]);
                        self.s.console.push(ConKind::Error, &l);
                    }
                }
                continue;
            }
            if self.loading.is_some() {
                return;
            }
            match self.items.pop_front() {
                Some(Item::Inline(src)) => {
                    let url = self.s.url.clone();
                    let used = self.run_script(view, &url, &src);
                    *budget = budget.saturating_sub(used);
                }
                Some(Item::Src(url)) => {
                    self.s.seq += 1;
                    let id = self.s.seq;
                    let mut headers = Headers::new();
                    headers.add("Accept", "text/plain, */*;q=0.5");
                    self.s.fx.fetch.push(WebReq { id, method: String::from("GET"), url: url.clone(), headers, body: Vec::new(), ..WebReq::default() });
                    self.loading = Some((id, url));
                }
                None => return,
            }
        }
    }

    /// a request of take_effects().fetch came back
    pub(crate) fn answer(&mut self, id: u64, r: Result<WebResp, String>) {
        if self.loading.as_ref().is_some_and(|l| l.0 == id) {
            if let Some((_, url)) = self.loading.take() {
                self.loaded = Some((url, r));
            }
        } else if self.s.fetches.contains_key(&id) {
            self.ready.push_back((id, r));
        }
    }

    // an answer of fetch to its callback
    fn run_answer(&mut self, view: &mut MdViewState, id: u64, r: Result<WebResp, String>) -> u64 {
        let Some(pin) = self.s.fetches.remove(&id) else { return 0 };
        let f = self.vm.pinned(pin);
        let vm = &mut self.vm;
        let made: Result<(Value, Value), Error> = (|| match &r {
            Ok(a) => {
                let m = vm.new_map()?;
                let k = vm.new_str("status")?;
                vm.map_set(m, k, Value::Int(a.status as i64))?;
                set_str(vm, m, "reason", &a.reason)?;
                let (k, h) = (vm.new_str("headers")?, headers_of(vm, &a.headers)?);
                vm.map_set(m, k, h)?;
                set_str(vm, m, "body", &String::from_utf8_lossy(&a.body))?;
                set_str(vm, m, "url", &a.url)?;
                let k = vm.new_str("redirects")?;
                vm.map_set(m, k, Value::Int(a.redirects as i64))?;
                Ok((Value::Nil, m))
            }
            Err(why) => Ok((vm.new_str(why)?, Value::Nil)),
        })();
        let used = match made {
            Ok((err, resp)) => self.invoke(view, f, &[err, resp]),
            Err(e) => {
                self.s.console.push(ConKind::Error, &e.msg);
                CALL_MIN
            }
        };
        self.vm.unpin(pin);
        used
    }

    // the timers due now, by time and number
    fn run_timers(&mut self, view: &mut MdViewState, budget: &mut u64) {
        let now = self.s.now;
        let mut due: Vec<(Ns, u64)> = self.s.timers.iter().filter(|(_, x)| x.at <= now).map(|(id, x)| (x.at, *id)).collect();
        due.sort_unstable();
        for (_, id) in due {
            if *budget == 0 {
                return;
            }
            // (one that an earlier one cancelled is gone)
            let Some(x) = self.s.timers.get_mut(&id) else { continue };
            let pin = x.pin;
            let once = x.period.is_none();
            match x.period {
                Some(p) => {
                    x.at = x.at.saturating_add(p);
                    if x.at <= now {
                        x.at = now.saturating_add(p);
                    }
                }
                None => {
                    self.s.timers.remove(&id);
                }
            }
            let f = self.vm.pinned(pin);
            let used = self.invoke(view, f, &[]);
            // (timer.every that cancels itself: timer.cancel took its pin)
            if once {
                self.vm.unpin(pin);
            }
            *budget = budget.saturating_sub(used);
        }
    }

    /// what is due at virtual time now (see the module header)
    pub(crate) fn pump(&mut self, view: &mut MdViewState, now: Ns, seq: &mut u64) {
        // (a past state became the present: the timers keep what they had left)
        if now < self.s.now {
            let back = self.s.now - now;
            for x in self.s.timers.values_mut() {
                x.at = x.at.saturating_sub(back);
            }
        }
        self.s.now = now;
        self.s.seq = *seq;
        let mut budget = FRAME_FUEL;
        self.run_items(view, &mut budget);
        while budget > 0 {
            let Some((id, r)) = self.ready.pop_front() else { break };
            let used = self.run_answer(view, id, r);
            budget = budget.saturating_sub(used);
        }
        self.run_timers(view, &mut budget);
        *seq = self.s.seq;
    }

    // an event from a node up to the document -> a handler stopped its default action; data: the
    // pairs of a submit
    fn dispatch(&mut self, view: &mut MdViewState, ty: u8, target: MdNode, data: &[(String, String)]) -> bool {
        if !self.s.handlers.keys().any(|k| k.1 == ty) {
            return false;
        }
        let mut path: Vec<u64> = Vec::new();
        let mut x = Some(target).filter(|n| !n.is_none());
        while let Some(n) = x {
            if n != view.doc().root() {
                path.push(n.raw());
            }
            x = view.doc().parent(n);
        }
        path.push(AT_DOCUMENT);
        self.s.event.serial += 1;
        self.s.event.prevented = false;
        self.s.event.stopped = false;
        let serial = self.s.event.serial;
        let (vm, s) = (&mut self.vm, &mut self.s);
        let made: Result<Value, Error> = (|| {
            let m = vm.new_map()?;
            set_str(vm, m, "kind", "event")?;
            set_str(vm, m, "type", ON_NAMES[ty as usize])?;
            let k = vm.new_str("serial")?;
            vm.map_set(m, k, Value::Int(serial))?;
            let (k, v) = (vm.new_str("target")?, node_map(vm, s, view.doc(), target)?);
            vm.map_set(m, k, v)?;
            if ty == ON_SUBMIT {
                let d = vm.new_map()?;
                for (name, val) in data {
                    set_str(vm, d, name, val)?;
                }
                let k = vm.new_str("data")?;
                vm.map_set(m, k, d)?;
            }
            Ok(m)
        })();
        let ev = match made {
            Ok(m) => m,
            Err(e) => {
                self.s.console.push(ConKind::Error, &e.msg);
                return false;
            }
        };
        let keep = self.vm.pin(ev);
        for at in path {
            let key = (at, ty);
            let Some(pins) = self.s.handlers.get(&key).cloned() else { continue };
            let cur = if at == AT_DOCUMENT { Ok(self.s.document) } else { node_map(&mut self.vm, &mut self.s, view.doc(), MdNode::from_raw(at)) };
            if let (Ok(c), Ok(k)) = (cur, self.vm.new_str("current")) {
                let _ = self.vm.map_set(ev, k, c);
            }
            for p in pins {
                // (one an earlier handler took away does not run)
                if !self.s.handlers.get(&key).is_some_and(|v| v.contains(&p)) {
                    continue;
                }
                let f = self.vm.pinned(p);
                self.invoke(view, f, &[ev]);
            }
            if self.s.event.stopped {
                break;
            }
        }
        self.vm.unpin(keep);
        self.s.event.serial += 1;
        self.s.event.prevented
    }

    /// what the view reported to the handlers -> the link and the form the page lets go
    pub(crate) fn events(&mut self, view: &mut MdViewState, ev: MdEv, now: Ns, seq: &mut u64) -> (Option<String>, Option<MdSubmit>) {
        self.s.now = self.s.now.max(now);
        self.s.seq = *seq;
        let (mut link, submit) = (ev.link, ev.submit);
        if let Some(n) = ev.click {
            if self.dispatch(view, ON_CLICK, n, &[]) {
                link = None;
            }
        }
        if let Some((node, kind)) = ev.field.and_then(|i| view.doc().fields().get(i)).map(|f| (f.node, f.kind)) {
            match kind {
                MF_TEXT | MF_PASSWORD | MF_TEXTAREA | MF_SELECT => {
                    self.dispatch(view, ON_INPUT, node, &[]);
                }
                MF_CHECKBOX | MF_RADIO => {
                    self.dispatch(view, ON_CLICK, node, &[]);
                    self.dispatch(view, ON_INPUT, node, &[]);
                }
                _ => {
                    self.dispatch(view, ON_CLICK, node, &[]);
                }
            }
        }
        let submit = submit.and_then(|s| self.submit(view, s));
        *seq = self.s.seq;
        (link, submit)
    }

    // a form on its way: its button's click, its submit -> what it sends as they left it
    fn submit(&mut self, view: &mut MdViewState, s: MdSubmit) -> Option<MdSubmit> {
        let form = view.doc().forms().get(s.form).map(|f| f.node).filter(|n| !n.is_none());
        let button = s.button.and_then(|b| view.doc().fields().get(b)).map(|f| f.node);
        if let Some(b) = button {
            if self.dispatch(view, ON_CLICK, b, &[]) {
                return None;
            }
        }
        if let Some(f) = form {
            if self.dispatch(view, ON_SUBMIT, f, &s.fields) {
                return None;
            }
        }
        view.update();
        let d = view.doc();
        let k = form.and_then(|f| d.forms().iter().position(|x| x.node == f)).unwrap_or(s.form);
        let Some(fm) = d.forms().get(k) else { return Some(s) };
        let b = button.and_then(|n| d.field_of(n));
        Some(MdSubmit { form: k, action: fm.action.clone(), method: fm.method.clone(), fields: view.form_data(k, b), button: b })
    }

    /// what the browser does for the page now
    pub(crate) fn take_effects(&mut self) -> Effects {
        core::mem::take(&mut self.s.fx)
    }

    /// when the next frame is needed, in ms of the wall clock (None: only with the clock's other
    /// frames); speed: the virtual clock's per mille of the wall clock, 0 while it stands
    pub(crate) fn wake(&self, now: Ns, speed: u32) -> Option<u32> {
        let waits = !self.ready.is_empty() || self.loaded.is_some() || (self.loading.is_none() && !self.items.is_empty());
        let next = self.s.timers.values().map(|x| x.at).min();
        if waits || next.is_some_and(|at| at <= now) {
            return Some(1);
        }
        if speed == 0 {
            return None;
        }
        let d = next?.saturating_sub(now);
        let ms = (d as u128 * 1000 / (speed as u128 * MS as u128)).min(3_600_000) as u32;
        Some(ms.max(1))
    }

    pub(crate) fn console(&self) -> &Console {
        &self.s.console
    }

    pub(crate) fn console_mut(&mut self) -> &mut Console {
        &mut self.s.console
    }

    pub(crate) fn clear_console(&mut self) {
        self.s.console.clear();
    }

    /// the src script on its way: its request
    pub(crate) fn src_request(&self) -> Option<u64> {
        self.loading.as_ref().map(|l| l.0)
    }

    /// the timers that wait, for a check
    pub(crate) fn timers(&self) -> usize {
        self.s.timers.len()
    }

    /// the requests this page has out, for a check
    pub(crate) fn running(&self) -> bool {
        self.loading.is_some() || !self.items.is_empty()
    }
}

/// what the browser adds to its numbers for a page's stream of chance
pub(crate) fn seed_of(project: u64, dev: u32, req: u64) -> u64 {
    project ^ (dev as u64) << 32 ^ req.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}
