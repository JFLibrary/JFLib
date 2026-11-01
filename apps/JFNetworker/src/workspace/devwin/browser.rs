// browser -- the Webbrowser on the Desktop of a host's device window: real small pages from the
// simulated network (milestones B1, B2, B3), shown with the library's document view instead of
// an HTML engine of our own
//
//   Browser::new()                    nothing open
//   program(n, ui, k, id, dev)        the program of device window k
//   report(n, dev, id, result)        Report::Web comes here (doc/console.rs routes it): the page
//                                     or image of that request
//   Browser::window_title()           the page's title and the program's name (the desktop's
//                                     window and taskbar)
//   Browser::close() -> ids           the window closes: its state goes, the requests to give up
//                                     (desktop.rs)
//
// Requests go to the simulator as Action::WebFetch through Cmd::Apply, so rewinding replays them;
// the engine's program (jfn_sim::webc) resolves the name, connects, follows redirects and reports
// the answer or an error kind.  While the simulation stands nothing moves: the status line says so.
// A request the engine no longer knows (a past state became the present) ends as an error page.
//
// A page: address(text) reads the address line (no scheme: http), https: and other schemes
// give a page of ours at once, without a request.  classify(type, body) picks the view: HTML
// (ui.doc_view on MdDoc::html), Markdown, images (a small page around the picture); text/plain
// and other text as preformatted text; what is no text a note with type and size.  An answer of
// 4xx or 5xx shows the server's HTML or Markdown when it sent one, else a page of ours with
// status and text, as for failures (error_page).  The title comes from <title>, else the first
// heading, else the file's name.  The status line tells status, type, size, time and redirects,
// or the target of the link under the pointer.  Texts are cut at MAX_SHOWN bytes.
//
// History: Hist of entries, each with its document (MdViewState: the place, the fields' values and
// the pictures stay); Back and Forward show them at once, without a request, also while the
// simulation stands; Reload asks again.  A fragment link scrolls (block_of: the element with that
// id or <a name>, in Markdown the heading with that slug) and is an entry of its own over the same
// document.  Reload of a POST result asks first (nothing goes out unasked: a second order is
// what the question is for) and sends it again on the answer.
//
// Images: every address comes from the app (MI_APP: a page from the network reads no file of the
// real machine): each one the view asks for (image_requests) is resolved against the page, data:
// decoded here, http: fetched and kept per window (cache, emptied by Reload), anything else
// is the alt text.  Requests of a page left go on and fill its document for Back.
//
// Forms: MdEv::submit gives form_nav(page, submit): GET with the query, POST with
// application/x-www-form-urlencoded, to the action resolved against the page; the answer is a new
// entry.  Links: MdEv::link resolved against the page (netpkt::uri).
//
// Page scripts (B3, pagescript.rs): a document's VM is Doc::page, made with the document from a
// server's HTML: it comes back with its entry on Back and Forward and runs nothing while it is not
// shown, Reload makes a new one.  Every pass runs what is due on the simulation's clock (page_pump);
// the events of the view go to its handlers, which decide about the link and the form; requests of
// fetch and of src scripts go like the pictures' (scripted: their document, a weak reference, for
// a document no entry has any more gets nothing).  A window with a page's timers waiting asks
// for a frame when the next one falls due (TIMER_PAGE).  The console's pane is below the page
// (Konsole, Ctrl+Shift+J; only for pages with scripts).
//
// Keys while its window is the active one: Enter in the address line, Ctrl+L the address line,
// Alt+Left / Alt+Right, F5, Esc stops a page on its way.
//
// Ids: id = the desktop's block + 0x30 (+ 0 the address, + 1 Öffnen, + 3 Zurück, + 4 Vor, + 5 Neu
// laden / Anhalten, + 6 / + 7 the question of a POST's reload); the page ID_BROWSER0 + 0x100 k; the
// console ID_PAGE0 + 0x80 k (+ 0 Konsole, + 1 its pane, + 2 Leeren).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::{Rc, Weak};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use jfn_sim::{Action, Cmd, Mode, WebErr, WebFail, WebReq, WebResp};
use netpkt::http::Headers;
use netpkt::uri::{self, Uri};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;

use super::pagescript::{console_pane, seed_of, ConSeen, PageScript};

/// entries of a window's history at most (the oldest go)
pub(crate) const HIST_MAX: usize = 30;
/// bytes of a page shown at most
const MAX_SHOWN: usize = 1 << 20;
/// bytes of pictures a window keeps; beyond that it starts over
const CACHE_MAX: usize = 32 << 20;
const ACCEPT_PAGE: &str = "text/html, text/markdown;q=0.9, text/plain;q=0.8, */*;q=0.5";
const ACCEPT_IMAGE: &str = "image/*, */*;q=0.5";
const FORM_TYPE: &str = "application/x-www-form-urlencoded";

/// what is shown of an answer
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Html,
    Markdown,
    Text,
    Image,
    Binary,
}

/// a window's history: entries in order and the one shown
pub(crate) struct Hist<T> {
    items: Vec<T>,
    pos: usize,
}

impl<T> Hist<T> {
    pub(crate) fn new() -> Hist<T> {
        Hist { items: Vec::new(), pos: 0 }
    }
    pub(crate) fn cur(&self) -> Option<&T> {
        self.items.get(self.pos)
    }
    pub(crate) fn cur_mut(&mut self) -> Option<&mut T> {
        self.items.get_mut(self.pos)
    }
    /// a new entry after the one shown: what lay ahead goes, the oldest beyond HIST_MAX too
    pub(crate) fn push(&mut self, v: T) {
        if !self.items.is_empty() {
            self.items.truncate(self.pos + 1);
        }
        self.items.push(v);
        if self.items.len() > HIST_MAX {
            self.items.remove(0);
        }
        self.pos = self.items.len() - 1;
    }
    /// the entry shown takes this place (Reload)
    pub(crate) fn replace(&mut self, v: T) {
        match self.items.get_mut(self.pos) {
            Some(e) => *e = v,
            None => self.push(v),
        }
    }
    pub(crate) fn can_back(&self) -> bool {
        self.pos > 0
    }
    pub(crate) fn can_forward(&self) -> bool {
        self.pos + 1 < self.items.len()
    }
    pub(crate) fn back(&mut self) -> bool {
        let can = self.can_back();
        if can {
            self.pos -= 1;
        }
        can
    }
    pub(crate) fn forward(&mut self) -> bool {
        let can = self.can_forward();
        if can {
            self.pos += 1;
        }
        can
    }
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
}

/// how a page is asked for
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Nav {
    pub method: String,
    pub url: String,
    pub body: Vec<u8>,
    /// the body's type
    pub ctype: String,
}

impl Nav {
    pub(crate) fn get(url: &str) -> Nav {
        Nav { method: String::from("GET"), url: String::from(url), ..Nav::default() }
    }
}

/// a document with what the window keeps of it
struct Doc {
    view: MdViewState,
    title: String,
    /// its URL: what images and links resolve against
    url: String,
    /// the status line
    status: String,
    /// its scripts (None: it has none)
    page: Option<Box<PageScript>>,
}

/// an entry of the history
struct Entry {
    url: String,
    nav: Nav,
    doc: Rc<RefCell<Doc>>,
    /// the first block in view when it was left
    top: usize,
}

/// a page on its way
struct Load {
    id: u64,
    nav: Nav,
    /// Reload: the answer takes the place of the entry shown
    replace: bool,
}

/// a picture on its way
struct Pic {
    /// as the document names it, and as it was asked for
    addr: String,
    url: String,
    doc: Rc<RefCell<Doc>>,
}

/// the program's state in a device window (the window's: closing it drops it)
pub(crate) struct Browser {
    addr: String,
    hist: Hist<Entry>,
    load: Option<Load>,
    pics: BTreeMap<u64, Pic>,
    cache: BTreeMap<String, Rc<Vec<u8>>>,
    /// the link under the pointer
    hover: String,
    /// Reload of a POST result waits for the answer
    ask: bool,
    /// the page takes the focus when it comes (from the address line, or from a field or a link
    /// of the page before it)
    focus_page: bool,
    /// the last request's number (it goes on after a close: the engine may still report)
    seq: u64,
    /// requests of page scripts: the document they came from
    scripted: BTreeMap<u64, Weak<RefCell<Doc>>>,
    /// the console's pane is open, the lines it showed
    console: bool,
    con_seen: ConSeen,
}

impl Browser {
    pub(crate) fn new() -> Browser {
        Browser { addr: String::from("http://"), hist: Hist::new(), load: None, pics: BTreeMap::new(), cache: BTreeMap::new(), hover: String::new(), ask: false, focus_page: false, seq: 0, scripted: BTreeMap::new(), console: false, con_seen: ConSeen::default() }
    }

    /// "Titel – Webbrowser", the program's name alone without a page
    pub(crate) fn window_title(&self, name: &str) -> String {
        match self.hist.cur().map(|e| e.doc.borrow().title.clone()) {
            Some(x) if !x.is_empty() => format!("{} \u{2013} {}", x, name),
            _ => String::from(name),
        }
    }

    /// the window closes: its state goes -> the requests still running
    pub(crate) fn close(&mut self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.pics.keys().copied().collect();
        ids.extend(self.load.as_ref().map(|l| l.id));
        ids.extend(self.scripted.keys().copied());
        let seq = self.seq;
        *self = Browser::new();
        self.seq = seq;
        ids
    }
}

// ---------------------------------------------------------------- what the parts decide

/// the URL the address line's text means: no scheme means http, what cannot be in a URL is escaped
pub(crate) fn address(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() {
        return String::new();
    }
    let known = t.contains("://") || ["about:", "data:", "mailto:", "javascript:"].iter().any(|p| t.len() > p.len() && t[..p.len()].eq_ignore_ascii_case(p));
    let mut u = if known { Uri::parse(t) } else { Uri::parse(&format!("http://{}", t)) }.encoded();
    if u.authority.is_some() && u.path.is_empty() {
        u.path.push('/');
    }
    u.text()
}

/// the view of an answer by its Content-Type; without one, or with an unknown one, by its bytes
pub(crate) fn classify(ctype: &str, body: &[u8]) -> Kind {
    let m = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let m = m.as_str();
    if m == "text/html" || m == "application/xhtml+xml" {
        Kind::Html
    } else if m == "text/markdown" || m == "text/x-markdown" {
        Kind::Markdown
    } else if m.starts_with("image/") {
        Kind::Image
    } else if m.starts_with("text/") || m == "application/json" || m == "application/xml" || m == "application/javascript" || m.ends_with("+json") || m.ends_with("+xml") {
        Kind::Text
    } else if text_like(body) {
        Kind::Text
    } else {
        Kind::Binary
    }
}

// valid UTF-8 without control characters but tabs and line ends
fn text_like(b: &[u8]) -> bool {
    !b.is_empty() && core::str::from_utf8(b).is_ok() && !b.iter().any(|&c| c < 9 || (c > 13 && c < 32))
}

// the text of a body by its charset (UTF-8 unless the type says Latin-1), cut at MAX_SHOWN
fn text_of(ctype: &str, body: &[u8]) -> (String, bool) {
    let cut = body.len() > MAX_SHOWN;
    let b = if cut { &body[..MAX_SHOWN] } else { body };
    let low = ctype.to_ascii_lowercase();
    let s = if low.contains("iso-8859-1") || low.contains("latin1") {
        b.iter().map(|&c| c as char).collect()
    } else {
        let b = b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b);
        String::from_utf8_lossy(b).into_owned()
    };
    (s, cut)
}

/// a text for HTML
pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// the request a form sends: the action resolved against the page; GET has the fields as its
/// query, POST as its body
pub(crate) fn form_nav(page: &str, s: &MdSubmit) -> Nav {
    let mut u = Uri::parse(page).join(&s.action).encoded();
    u.fragment = None;
    let data = uri::form_encode(&s.fields);
    if s.method.eq_ignore_ascii_case("post") {
        return Nav { method: String::from("POST"), url: u.text(), body: data.into_bytes(), ctype: String::from(FORM_TYPE) };
    }
    u.query = Some(data);
    Nav::get(&u.text())
}

/// the bytes of a data: address
pub(crate) fn data_url(addr: &str) -> Option<Vec<u8>> {
    let rest = addr.get(5..).filter(|_| addr.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:")))?;
    let (meta, data) = rest.split_once(',')?;
    if meta.to_ascii_lowercase().ends_with(";base64") {
        let clean: Vec<u8> = uri::percent_decode(data).into_iter().filter(|c| !c.is_ascii_whitespace()).collect();
        netpkt::imf::b64_decode(&clean)
    } else {
        Some(uri::percent_decode(data))
    }
}

// a heading's slug as GitHub writes it
fn slug(s: &str) -> String {
    let low = s.trim().to_lowercase();
    low.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_').map(|c| if c == ' ' { '-' } else { c }).collect()
}

/// the block a fragment leads to: the element with that id or name (the first block inside it,
/// else the block around it), in Markdown the heading with that slug
pub(crate) fn block_of(d: &MdDoc, frag: &str) -> Option<usize> {
    let name = uri::percent_text(frag);
    if name.is_empty() {
        return Some(0);
    }
    let found = d.by_id(&name).or_else(|| d.select_first(&format!("a[name=\"{}\"]", name.replace('"', ""))));
    if let Some(n) = found {
        let inside = |b: &MdBlock| {
            let mut m = Some(b.node);
            while let Some(x) = m {
                if x == n {
                    return true;
                }
                m = d.parent(x);
            }
            false
        };
        if let Some(i) = d.blocks().iter().position(inside) {
            return Some(i);
        }
        let mut m = d.parent(n);
        while let Some(x) = m {
            if let Some(i) = d.blocks().iter().position(|b| b.node == x) {
                return Some(i);
            }
            m = d.parent(x);
        }
    }
    let want = slug(&name);
    d.blocks().iter().position(|b| b.kind == MB_HEADING && slug(&b.text) == want)
}

// the title of a document: <title>, else its first heading
fn title_of(d: &MdDoc) -> String {
    if let Some(n) = d.select_first("title") {
        let x = d.text_of(n);
        let x = x.trim();
        if !x.is_empty() {
            return String::from(x);
        }
    }
    d.blocks().iter().find(|b| b.kind == MB_HEADING).map(|b| String::from(b.text.trim())).unwrap_or_default()
}

// the last part of a URL's path, for what has no title
fn file_of(url: &str) -> String {
    let u = Uri::parse(url);
    let name = u.path.rsplit('/').next().unwrap_or("");
    if name.is_empty() { u.host().map(String::from).unwrap_or_default() } else { uri::percent_text(name) }
}

// a page of ours: a heading, its paragraphs, the address
fn own_page(title: &str, paras: &[String], url: &str) -> String {
    let mut s = format!("<title>{}</title><div style=\"border-left: 4px solid #B0413E; padding: 2px 0 2px 12px\"><h2>{}</h2>", esc(title), esc(title));
    for p in paras.iter().filter(|p| !p.is_empty()) {
        s.push_str(&format!("<p>{}</p>", esc(p)));
    }
    if !url.is_empty() {
        s.push_str(&format!("<p><code>{}</code></p>", esc(url)));
    }
    s.push_str("</div>");
    s
}

fn host_of(url: &str) -> String {
    Uri::parse(url).host().map(String::from).unwrap_or_default()
}

/// the page of a failure: its title and HTML (error pages are our own design and words)
pub(crate) fn error_page(kind: WebErr, url: &str, why: &str) -> (String, String) {
    let host = host_of(url);
    let port = Uri::parse(url).port().map_or_else(|| String::from("80"), |p| p.to_string());
    let (title, reason, hint) = match kind {
        WebErr::Refused => (t("Seite nicht erreichbar"), tf("{} lehnt die Verbindung ab: auf Port {} lauscht kein Dienst.", &[&host, &port]), t("Läuft der Webserver auf dem Gerät? Der Reiter Dienste und „ss -tln“ zeigen, was lauscht.")),
        WebErr::Timeout => (t("Seite nicht erreichbar"), tf("{} hat nicht rechtzeitig geantwortet.", &[&host]), t("Kommt das Gerät per ping an? Der Simulationsmodus zeigt, wo die Pakete bleiben.")),
        WebErr::NoName => (t("Seite nicht erreichbar"), tf("Den Namen {} kann dieses Gerät nicht auflösen.", &[&host]), t("Stimmt der Name? Ein Name braucht einen Eintrag in /etc/hosts oder einen Namensserver in /etc/resolv.conf.")),
        WebErr::NoRoute => (t("Seite nicht erreichbar"), tf("Zu {} führt von hier kein Weg.", &[&host]), t("Hat das Gerät eine Adresse und ein Gateway, und sind die Kabel gesteckt?")),
        WebErr::Broken => (t("Seite nicht erreichbar"), tf("Die Verbindung zu {} brach mitten in der Antwort ab.", &[&host]), ""),
        WebErr::Stopped => (t("Seite nicht erreichbar"), String::from(t("Die Anfrage endete, weil das Gerät oder sein Dienst angehalten hat.")), ""),
        WebErr::TooManyRedirects => (t("Zu viele Weiterleitungen"), tf("{} leitet immer weiter.", &[&host]), t("Oft zeigen zwei Seiten aufeinander; curl -L -v im Terminal zeigt die Kette.")),
        WebErr::Https => (t("Verschlüsselte Seite"), String::from(t("Verschlüsselte Seiten (https) kann dieser Browser noch nicht öffnen: sie kommen mit einer späteren Ausbaustufe.")), t("Dieselbe Seite über http:// geht, wenn der Server dort antwortet.")),
        WebErr::BadUrl => (t("Adresse kann nicht geöffnet werden"), String::from(t("Dieser Browser öffnet http-Adressen im simulierten Netz.")), ""),
    };
    let html = own_page(title, &[reason, String::from(why), String::from(hint)], url);
    (String::from(title), html)
}

// the document of a page in HTML
fn html_doc(html: &str, title: String, url: &str, status: String) -> Doc {
    let mut view = MdViewState::new();
    view.images = MI_APP;
    view.label = String::from(t("Seite"));
    view.set_html(html);
    let title = if title.is_empty() { title_of(view.doc()) } else { title };
    Doc { view, title, url: String::from(url), status, page: None }
}

// the status line of an answer
fn status_line(r: &WebResp, ctype: &str, cut: bool) -> String {
    let mut parts = alloc::vec![format!("{} {}", r.status, r.reason)];
    let kind = ctype.split(';').next().unwrap_or("").trim();
    if !kind.is_empty() {
        parts.push(String::from(kind));
    }
    parts.push(tf("{} Bytes", &[&r.body.len().to_string()]));
    parts.push(tr::msg(&jfn_model::msg::dur(r.took as i64)));
    match r.redirects {
        0 => {}
        1 => parts.push(String::from(t("eine Weiterleitung"))),
        k => parts.push(tf("{} Weiterleitungen", &[&k.to_string()])),
    }
    if cut {
        parts.push(String::from(t("gekürzt")));
    }
    parts.join(" \u{00B7} ")
}

// the document of an answer; seed: for its scripts' chance
fn doc_of(r: &WebResp, seed: u64) -> Doc {
    let ctype = r.headers.get("content-type").unwrap_or("");
    let mut kind = classify(ctype, &r.body);
    if r.body.is_empty() {
        kind = Kind::Binary;
    }
    let (text, cut) = if matches!(kind, Kind::Binary | Kind::Image) { (String::new(), false) } else { text_of(ctype, &r.body) };
    let status = status_line(r, ctype, cut);
    if r.status >= 300 && !matches!(kind, Kind::Html | Kind::Markdown) {
        // no page of the server's own: ours, with what it said
        let title = if r.status >= 500 { t("Der Server meldet einen Fehler") } else if r.status >= 400 { t("Die Seite gibt es so nicht") } else { t("Keine Seite") };
        let says = tf("{} antwortet mit {} {}.", &[&host_of(&r.url), &r.status.to_string(), &r.reason]);
        let html = own_page(title, &[says, String::from(text.trim())], &r.url);
        return html_doc(&html, String::from(title), &r.url, status);
    }
    match kind {
        Kind::Html => {
            let mut d = html_doc(&text, String::new(), &r.url, status);
            d.page = PageScript::of(&d.view, &text, &r.url, seed);
            d
        }
        Kind::Markdown => {
            let mut view = MdViewState::new();
            view.images = MI_APP;
            view.label = String::from(t("Seite"));
            view.set_markdown(&text);
            let title = title_of(view.doc());
            Doc { view, title, url: r.url.clone(), status, page: None }
        }
        Kind::Text => {
            let html = format!("<pre>{}</pre>", esc(&text));
            html_doc(&html, file_of(&r.url), &r.url, status)
        }
        Kind::Image => {
            let name = file_of(&r.url);
            let html = format!("<p><img src=\"{}\" alt=\"{}\"></p>", esc(&r.url), esc(&name));
            let mut d = html_doc(&html, name, &r.url, status);
            d.view.set_image(&r.url, Some(&r.body));
            d
        }
        Kind::Binary => {
            let title = if r.body.is_empty() { t("Die Antwort hat keinen Inhalt") } else { t("Diese Datei zeigt der Browser nicht an") };
            let what = if r.body.is_empty() { String::new() } else { tf("Typ {}, {} Bytes.", &[if ctype.is_empty() { t("unbekannt") } else { ctype }, &r.body.len().to_string()]) };
            let html = own_page(title, &[what], &r.url);
            html_doc(&html, String::from(title), &r.url, status)
        }
    }
}

// ---------------------------------------------------------------- the outcome of a request

/// a report of the engine for this device's browser
pub(crate) fn report(n: &mut Net, dev: u32, id: u64, result: Result<WebResp, WebFail>) {
    let seed = seed_of(n.doc.sim.engine().seed(), dev, id);
    let Some(d) = n.devwin.desk_of(dev) else { return };
    let s = &mut d.web;
    if let Some(w) = s.scripted.remove(&id) {
        if let Some(doc) = w.upgrade() {
            if let Some(p) = doc.borrow_mut().page.as_mut() {
                p.answer(id, result.map_err(|f| tr::msg(&f.msg)));
            }
        }
        return;
    }
    if s.load.as_ref().is_some_and(|l| l.id == id) {
        let load = s.load.take();
        if let Some(l) = load {
            let (url, doc) = match &result {
                Ok(r) => (r.url.clone(), doc_of(r, seed)),
                Err(f) => {
                    let why = tr::msg(&f.msg);
                    let (title, html) = error_page(f.kind, &f.url, &why);
                    (f.url.clone(), html_doc(&html, title, &f.url, String::from(t("Keine Antwort"))))
                }
            };
            arrive(s, l.nav, l.replace, url, doc);
        }
        return;
    }
    let Some(p) = s.pics.remove(&id) else { return };
    let bytes = match result {
        Ok(r) if r.status < 300 => Some(r.body),
        _ => None,
    };
    if let Some(b) = &bytes {
        if s.cache.values().map(|x| x.len()).sum::<usize>() + b.len() > CACHE_MAX {
            s.cache.clear();
        }
        s.cache.insert(p.url.clone(), Rc::new(b.clone()));
    }
    p.doc.borrow_mut().view.set_image(&p.addr, bytes.as_deref());
}

// a document arrived: a new entry, or the place of the one shown
fn arrive(s: &mut Browser, nav: Nav, replace: bool, url: String, mut doc: Doc) {
    keep_top(s);
    // a fragment of the final URL: the view starts there
    let frag = Uri::parse(&url).fragment;
    if let Some(i) = frag.and_then(|f| block_of(doc.view.doc(), &f)) {
        doc.view.scroll_to_block(i);
    }
    let top = doc.view.first_visible().0;
    let e = Entry { url: url.clone(), nav, doc: Rc::new(RefCell::new(doc)), top };
    if replace {
        s.hist.replace(e);
    } else {
        s.hist.push(e);
    }
    s.addr = url;
    s.ask = false;
    s.hover.clear();
}

// the entry being left remembers its place
fn keep_top(s: &mut Browser) {
    if let Some(e) = s.hist.cur_mut() {
        let top = e.doc.borrow().view.first_visible().0;
        e.top = top;
    }
}

// the entry the history moved to is shown where it was left
fn show_entry(s: &mut Browser) {
    if let Some(e) = s.hist.cur() {
        s.addr = e.url.clone();
        e.doc.borrow_mut().view.scroll_to_block(e.top);
    }
    s.ask = false;
    s.hover.clear();
    s.focus_page = true;
}

// ---------------------------------------------------------------- what the window asks for

// the page on its way is given up
fn drop_load(n: &mut Net, k: usize, dev: u32) {
    let Some(w) = n.devwin.get(k) else { return };
    if let Some(l) = w.desk.web.load.take() {
        let _ = n.doc.sim.send(Cmd::Apply(Action::WebCancel { dev, id: l.id }));
    }
}

// a request for a page, or at once a page of ours when it cannot be asked for
fn go(n: &mut Net, k: usize, dev: u32, nav: Nav, replace: bool) {
    drop_load(n, k, dev);
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    s.ask = false;
    let u = Uri::parse(&nav.url);
    let local = match u.scheme.as_deref() {
        Some("http") => None,
        Some("https") => Some(WebErr::Https),
        _ => Some(WebErr::BadUrl),
    };
    if let Some(kind) = local {
        let (title, html) = error_page(kind, &nav.url, "");
        let url = nav.url.clone();
        let doc = html_doc(&html, title, &url, String::new());
        arrive(s, nav, replace, url, doc);
        return;
    }
    s.seq += 1;
    let id = s.seq;
    s.addr = nav.url.clone();
    let mut headers = Headers::new();
    headers.add("Accept", ACCEPT_PAGE);
    if !nav.ctype.is_empty() {
        headers.add("Content-Type", &nav.ctype);
    }
    let req = WebReq { id, method: nav.method.clone(), url: nav.url.clone(), headers, body: nav.body.clone() };
    s.load = Some(Load { id, nav, replace });
    let r = n.doc.sim.send(Cmd::Apply(Action::WebFetch { dev, req }));
    if let Err(e) = r {
        let why = tr::msg(&e.msg());
        let Some(w) = n.devwin.get(k) else { return };
        let s = &mut w.desk.web;
        if let Some(l) = s.load.take() {
            let url = l.nav.url.clone();
            let (title, html) = error_page(WebErr::Stopped, &url, &why);
            let doc = html_doc(&html, title, &url, String::from(t("Keine Antwort")));
            arrive(s, l.nav, l.replace, url, doc);
        }
    }
}

// a link of the page, or the address of a form's answer: a fragment of this page scrolls
fn follow(n: &mut Net, k: usize, dev: u32, target: &str) {
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    let Some((page, doc, nav)) = s.hist.cur().map(|e| (e.url.clone(), e.doc.clone(), e.nav.clone())) else { return };
    let u = Uri::parse(&page).join(target.trim()).encoded();
    let same = Uri { fragment: None, ..u.clone() }.text() == Uri { fragment: None, ..Uri::parse(&page) }.text();
    match &u.fragment {
        Some(f) if same => {
            keep_top(s);
            let top = block_of(doc.borrow().view.doc(), f);
            if let Some(i) = top {
                doc.borrow_mut().view.scroll_to_block(i);
            }
            let top = top.unwrap_or_else(|| doc.borrow().view.first_visible().0);
            let url = u.text();
            s.addr = url.clone();
            s.hist.push(Entry { url, nav, doc, top });
        }
        _ => go(n, k, dev, Nav::get(&u.text()), false),
    }
}

// the address line: the same address again is a reload, a new one a new entry
fn open_address(n: &mut Net, k: usize, dev: u32) {
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    let url = address(&s.addr);
    if url.is_empty() || url == "http://" {
        return;
    }
    s.focus_page = true;
    let cur = s.hist.cur().map(|e| e.url.clone());
    if cur.as_deref() == Some(url.as_str()) {
        s.cache.clear();
        go(n, k, dev, Nav::get(&url), true);
    } else {
        go(n, k, dev, Nav::get(&url), false);
    }
}

// F5: ask again; the result of a POST asks the user first (`sure`: the question's answer)
fn reload(n: &mut Net, k: usize, dev: u32, sure: bool) {
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    let Some(nav) = s.hist.cur().map(|e| e.nav.clone()) else { return };
    if nav.method == "POST" && !sure {
        s.ask = true;
        return;
    }
    s.cache.clear();
    go(n, k, dev, nav, true);
}

// what the document wants: the pictures it waits for
fn fetch_pictures(n: &mut Net, k: usize, dev: u32, doc: &Rc<RefCell<Doc>>, wanted: Vec<String>) {
    for addr in wanted {
        let page = doc.borrow().url.clone();
        let abs = Uri::parse(&page).join(&addr).encoded();
        if abs.scheme.as_deref() == Some("data") {
            let bytes = data_url(&addr);
            doc.borrow_mut().view.set_image(&addr, bytes.as_deref());
            continue;
        }
        if abs.scheme.as_deref() != Some("http") {
            doc.borrow_mut().view.set_image(&addr, None);
            continue;
        }
        let url = Uri { fragment: None, ..abs }.text();
        let Some(w) = n.devwin.get(k) else { return };
        let s = &mut w.desk.web;
        if let Some(b) = s.cache.get(&url).cloned() {
            doc.borrow_mut().view.set_image(&addr, Some(&b));
            continue;
        }
        s.seq += 1;
        let id = s.seq;
        let mut headers = Headers::new();
        headers.add("Accept", ACCEPT_IMAGE);
        let req = WebReq { id, method: String::from("GET"), url: url.clone(), headers, body: Vec::new() };
        s.pics.insert(id, Pic { addr: addr.clone(), url, doc: doc.clone() });
        if n.doc.sim.send(Cmd::Apply(Action::WebFetch { dev, req })).is_err() {
            if let Some(w) = n.devwin.get(k) {
                w.desk.web.pics.remove(&id);
            }
            doc.borrow_mut().view.set_image(&addr, None);
        }
    }
}

// Anhalten: the page and the pictures on their way are given up
fn stop(n: &mut Net, k: usize, dev: u32) {
    drop_load(n, k, dev);
    let Some(w) = n.devwin.get(k) else { return };
    let pics = core::mem::take(&mut w.desk.web.pics);
    for (id, p) in pics {
        p.doc.borrow_mut().view.set_image(&p.addr, None);
        let _ = n.doc.sim.send(Cmd::Apply(Action::WebCancel { dev, id }));
    }
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    if let Some(e) = s.hist.cur() {
        s.addr = e.url.clone();
    }
}

// a request the engine no longer knows (a past state became the present, the device changed)
// ends here: nothing will report it any more
fn lost_requests(n: &mut Net, k: usize, dev: u32) {
    let active = n.doc.sim.engine().web_client(dev).map(|v| v.active).unwrap_or_default();
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    if s.load.as_ref().is_some_and(|l| !active.contains(&l.id)) {
        if let Some(l) = s.load.take() {
            let url = l.nav.url.clone();
            let (title, html) = error_page(WebErr::Stopped, &url, "");
            let doc = html_doc(&html, title, &url, String::from(t("Keine Antwort")));
            arrive(s, l.nav, l.replace, url, doc);
        }
    }
    let gone: Vec<u64> = s.pics.keys().copied().filter(|id| !active.contains(id)).collect();
    for id in gone {
        if let Some(p) = s.pics.remove(&id) {
            p.doc.borrow_mut().view.set_image(&p.addr, None);
        }
    }
    let gone: Vec<u64> = s.scripted.keys().copied().filter(|id| !active.contains(id)).collect();
    for id in gone {
        if let Some(doc) = s.scripted.remove(&id).and_then(|w| w.upgrade()) {
            if let Some(p) = doc.borrow_mut().page.as_mut() {
                p.answer(id, Err(String::from(t("Die Anfrage endete, weil das Gerät oder sein Dienst angehalten hat."))));
            }
        }
    }
}

// the scripts of the document shown: what is due now, and a frame for what comes next
fn page_pump(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let now = n.doc.sim.engine().now();
    let past = n.doc.sim.view_at().is_some();
    let speed = match n.doc.sim.mode() {
        Mode::Realtime { speed_pm } if !past => speed_pm,
        _ => 0,
    };
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    let Some((doc, url)) = s.hist.cur().map(|e| (e.doc.clone(), e.url.clone())) else { return };
    {
        let mut d = doc.borrow_mut();
        let d = &mut *d;
        let Some(p) = d.page.as_mut() else { return };
        p.set_location(&url);
        // (while the view stands in the past no script runs)
        if !past {
            p.pump(&mut d.view, now, &mut s.seq);
        }
        if let Some(ms) = p.wake(now, speed) {
            ui.timer(TIMER_PAGE + k as u32 % 4, ms);
        }
    }
    page_effects(n, ui, k, dev, &doc);
}

// what the scripts of a document asked for
fn page_effects(n: &mut Net, ui: &mut Ui, k: usize, dev: u32, doc: &Rc<RefCell<Doc>>) {
    let fx = match doc.borrow_mut().page.as_mut() {
        Some(p) => p.take_effects(),
        None => return,
    };
    if fx.changed {
        let mut d = doc.borrow_mut();
        let x = title_of(d.view.doc());
        if !x.is_empty() && x != d.title {
            d.title = x;
            ui.repass();
        }
    }
    for req in fx.fetch {
        let id = req.id;
        if let Some(w) = n.devwin.get(k) {
            w.desk.web.scripted.insert(id, Rc::downgrade(doc));
        }
        if let Err(e) = n.doc.sim.send(Cmd::Apply(Action::WebFetch { dev, req })) {
            let why = tr::msg(&e.msg());
            if let Some(w) = n.devwin.get(k) {
                w.desk.web.scripted.remove(&id);
            }
            if let Some(p) = doc.borrow_mut().page.as_mut() {
                p.answer(id, Err(why));
            }
            ui.repass();
        }
    }
    if let Some(target) = fx.nav {
        follow(n, k, dev, &target);
        ui.repass();
    } else if fx.reload {
        reload(n, k, dev, false);
        ui.repass();
    }
}

// ---------------------------------------------------------------- the window

enum Act {
    Open,
    Back,
    Forward,
    Reload,
    Stop,
    Resend,
    KeepPage,
    Link(String),
    Submit(MdSubmit),
}

/// the Webbrowser of device window k
pub(crate) fn program(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    lost_requests(n, k, dev);
    page_pump(n, ui, k, dev);
    let mode = n.doc.sim.mode();
    let now = n.doc.sim.engine().now();
    let page_id = ID_BROWSER0 + 0x100 * k as u32;
    let con_id = ID_PAGE0 + 0x80 * k as u32;
    let Some(w) = n.devwin.get(k) else { return };
    let front = w.desk.front == super::desktop::Prog::Browser.key();
    let s = &mut w.desk.web;
    let scripts = s.hist.cur().is_some_and(|e| e.doc.borrow().page.is_some());
    let mut act: Option<Act> = None;
    if front {
        if ui.capture_shortcut(VK_LEFT, MOD_ALT) {
            act = Some(Act::Back);
        } else if ui.capture_shortcut(VK_RIGHT, MOD_ALT) {
            act = Some(Act::Forward);
        } else if ui.capture_shortcut(VK_F5, 0) {
            act = Some(Act::Reload);
        } else if s.load.is_some() && ui.capture_shortcut(VK_ESCAPE, 0) {
            act = Some(Act::Stop);
        } else if ui.capture_shortcut(b'L' as u32, MOD_CTRL) {
            ui.focus(id);
            let text = s.addr.clone();
            ui.textbox_select(id, &text, 0, text.len());
        } else if scripts && ui.capture_shortcut(b'J' as u32, MOD_CTRL | MOD_SHIFT) {
            s.console = !s.console;
            ui.repass();
        }
    }
    let loading = s.load.is_some();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
    let flag = |on: bool| ST_ICON_ONLY | ST_SUBTLE | if on { 0 } else { ST_DISABLED };
    ui.next_icon(IC_BACK);
    ui.next_flags(flag(s.hist.can_back()));
    if ui.button(id + 3, t("Zurück")) & EV_ACT != 0 {
        act = Some(Act::Back);
    }
    ui.tooltip(t("Zurück (Alt+Links)"));
    ui.next_icon(IC_FORWARD);
    ui.next_flags(flag(s.hist.can_forward()));
    if ui.button(id + 4, t("Vor")) & EV_ACT != 0 {
        act = Some(Act::Forward);
    }
    ui.tooltip(t("Vor (Alt+Rechts)"));
    ui.next_icon(if loading { IC_STOP } else { IC_REFRESH });
    ui.next_flags(flag(loading || s.hist.cur().is_some()));
    if ui.button(id + 5, if loading { t("Anhalten") } else { t("Neu laden") }) & EV_ACT != 0 {
        act = Some(if loading { Act::Stop } else { Act::Reload });
    }
    ui.tooltip(if loading { t("Anhalten (Esc)") } else { t("Neu laden (F5)") });
    ui.next_size(-1, 0);
    let ev = ui.textbox(id, Some("http://10.0.0.10/"), &mut s.addr, 2000);
    if ev & EV_COMMIT != 0 && ui.focused() == id {
        act = Some(Act::Open);
    }
    ui.tooltip(t("Adresse (Strg+L)"));
    if ui.button(id + 1, t("Öffnen")) & EV_ACT != 0 {
        act = Some(Act::Open);
    }
    if scripts {
        let mut on = s.console as i32;
        ui.next_icon(crate::workspace::icons::JN_TERMINAL);
        ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
        if ui.toggle_button(con_id, t("Konsole"), &mut on) & EV_ACT != 0 {
            s.console = on != 0;
            ui.repass();
        }
        ui.tooltip(t("Konsole der Seitenskripte (Strg+Umschalt+J)"));
    }
    ui.box_end();
    // Reload of a POST result: the question above the page
    if s.ask {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_CARD | pad(6), 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, t("Diese Seite ist die Antwort auf ein Formular. Neu laden sendet es noch einmal."));
        if ui.accent_button(id + 6, t("Erneut senden")) & EV_ACT != 0 {
            act = Some(Act::Resend);
        }
        if ui.button(id + 7, t("Abbrechen")) & EV_ACT != 0 {
            act = Some(Act::KeepPage);
        }
        ui.box_end();
    }
    // the page
    let doc = s.hist.cur().map(|e| e.doc.clone());
    let mut wanted = Vec::new();
    match &doc {
        Some(d) => {
            ui.next_size(-1, -1);
            let mut d = d.borrow_mut();
            let mut ev = ui.doc_view(page_id, &mut d.view, 0);
            wanted = d.view.image_requests();
            s.hover = ev.hover.take().unwrap_or_default();
            // the page's handlers decide about its link and its form
            let dd = &mut *d;
            if let Some(p) = dd.page.as_mut() {
                let (link, submit) = p.events(&mut dd.view, ev.clone(), now, &mut s.seq);
                (ev.link, ev.submit) = (link, submit);
            }
            if let Some(l) = ev.link {
                act = Some(Act::Link(l));
            }
            if let Some(f) = ev.submit {
                act = Some(Act::Submit(f));
            }
            // the focus was on the address line or on a field of the page before: it goes to the
            // page; on anything else (a button, another window) it stays
            if s.focus_page && !loading {
                s.focus_page = false;
                let f = ui.focused();
                if f == id || f & 0xFFFF_0000 == md_field_id(page_id, 0) & 0xFFFF_0000 {
                    ui.focus(page_id);
                }
            }
        }
        None => {
            ui.next_size(-1, -1);
            ui.box_begin(page_id, BOX_V | BOX_CENTER | BOX_XCENTER | BOX_CARD | pad(12), 4);
            ui.empty_state(crate::workspace::icons::JN_APP_BROWSER, t("Webbrowser"), t("Eine Adresse eingeben und „Öffnen“: Seiten kommen von den Webservern und Skripten im simulierten Netz."));
            ui.box_end();
        }
    }
    // the status line
    let waiting = doc.as_ref().map_or(0, |d| s.pics.values().filter(|p| Rc::ptr_eq(&p.doc, d)).count());
    let line = if !s.hover.is_empty() {
        s.hover.clone()
    } else if let Some(l) = &s.load {
        let host = host_of(&l.nav.url);
        match mode {
            Mode::Realtime { .. } => tf("Lädt von {} …", &[&host]),
            Mode::Paused => tf("Lädt von {} … Die Simulation ist angehalten: die Antwort kommt, wenn sie weiterläuft.", &[&host]),
            Mode::Simulation => tf("Lädt von {} … Im Simulationsmodus geht die Anfrage mit den Schritten voran.", &[&host]),
        }
    } else if let Some(d) = &doc {
        let mut st = d.borrow().status.clone();
        let errors = d.borrow().page.as_ref().map_or(0, |p| p.console().errors());
        if errors > 0 && !s.console {
            st = tf("{} · Skriptfehler: {} (Konsole)", &[&st, &errors.to_string()]);
        }
        match waiting {
            0 => st,
            1 => tf("{} · ein Bild lädt noch …", &[&st]),
            w => tf("{} · {} Bilder laden noch …", &[&st, &w.to_string()]),
        }
    } else {
        String::new()
    };
    // the console of the page's scripts
    if scripts && s.console {
        if let Some(d) = &doc {
            let mut d = d.borrow_mut();
            if let Some(p) = d.page.as_mut() {
                if console_pane(ui, con_id + 1, p.console(), &mut s.con_seen) {
                    p.clear_console();
                    ui.repass();
                }
            }
        }
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &line);
    // what the widgets and keys asked for, after them
    if let Some(d) = &doc {
        if !wanted.is_empty() {
            fetch_pictures(n, k, dev, d, wanted);
        }
        page_effects(n, ui, k, dev, d);
    }
    let Some(a) = act else { return };
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    match a {
        Act::Open => open_address(n, k, dev),
        Act::Back => {
            keep_top(s);
            if s.hist.back() {
                show_entry(s);
                drop_load(n, k, dev);
            }
        }
        Act::Forward => {
            keep_top(s);
            if s.hist.forward() {
                show_entry(s);
                drop_load(n, k, dev);
            }
        }
        Act::Reload => reload(n, k, dev, false),
        Act::Stop => stop(n, k, dev),
        Act::Resend => reload(n, k, dev, true),
        Act::KeepPage => s.ask = false,
        Act::Link(l) => follow(n, k, dev, &l),
        Act::Submit(f) => {
            s.focus_page = true;
            let page = s.hist.cur().map(|e| e.url.clone()).unwrap_or_default();
            go(n, k, dev, form_nav(&page, &f), false);
        }
    }
    ui.repass();
}
