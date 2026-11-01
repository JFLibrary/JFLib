// browser -- the Webbrowser on the Desktop of a host's device window: real small pages from the
// simulated network (milestones B1 to B4), shown with the library's document view instead of
// an HTML engine of our own
//
//   Browser::new()                    nothing open
//   program(n, ui, k, id, dev)        the program of device window k
//   report(n, dev, id, result)        Report::Web comes here (doc/console.rs routes it): the page
//                                     or image of that request
//   Browser::window_title()           the page's title and the program's name (the desktop's
//                                     window and taskbar)
//   Browser::close() -> ids           the window closes: its state goes but for the cookies that
//                                     last, the requests to give up (desktop.rs)
//
// Requests go to the simulator as Action::WebFetch through Cmd::Apply, so rewinding replays them;
// the engine's program (jfn_sim::webc) resolves the name, connects, follows redirects and reports
// the answer or an error kind.  While the simulation stands nothing moves: the status line says so.
// A request the engine no longer knows (a past state became the present) ends as an error page.
//
// A page: address(text) reads the address line (no scheme: http), schemes but http and https
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
// Style sheets (B4): every <link rel="stylesheet"> the document names (doc.sheet_links(), also one a
// script adds) is fetched once like a picture, a text/css answer goes to the document (doc.set_sheet)
// and counts where its link stands among the <style> elements; a sheet that fails is a console line and
// the page stays as it is.  Cookies (B4): the jar is the window's (netpkt::cookie, an Arc); every
// request carries a snapshot of it with who asks (stamp), the engine reads the Cookie header of each
// hop from that, and the Set-Cookie headers of all hops come back in the report and go into the jar in
// their order (store_cookies).  HTTPS (B4): a certificate with faults brings a warning page of ours with
// what is wrong; Trotzdem fortfahren remembers that certificate (SHA-256 of the leaf, window state) and
// asks again; the lock beside the address opens a card with the certificate.  Anhalten ends all that is
// on its way: the page, pictures, sheets, fetch and src requests.
//
// The developer view (devtools.rs): F12 or its button; Konsole, Netzwerk, Elemente, Cookies; below the
// page, with a splitter.  Each document has its own log (Doc::net) and tree.
//
// Keys while its window is the active one: Enter in the address line, Ctrl+L the address line,
// Alt+Left / Alt+Right, F5, F12, Esc stops what is on its way.
//
// Ids: id = the desktop's block + 0x30 (+ 0 the address, + 1 Öffnen, + 3 Zurück, + 4 Vor, + 5 Neu
// laden / Anhalten, + 6 / + 7 the question of a POST's reload); the page ID_BROWSER0 + 0x100 k; the
// console ID_PAGE0 + 0x80 k (+ 0 Konsole, + 1 its pane, + 2 Leeren); the rest devtools.rs.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::collections::BTreeSet;
use alloc::rc::{Rc, Weak};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;

use jfn_sim::webc::{TlsInfo, WebHop};
use jfn_sim::{Action, Cmd, Mode, WebErr, WebFail, WebReq, WebResp};
use netpkt::cookie::{site_of, Jar, Origin};
use netpkt::http::Headers;
use netpkt::uri::{self, Uri};
use netpkt::x509::{F_BAD_SIGNATURE, F_EXPIRED, F_NAME, F_NOT_YET, F_SELF_SIGNED, F_UNKNOWN_ISSUER};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;

use super::devtools::{self, date_text, net_add, net_answer, net_stopped, pending_row, rows_of, Dev, NetRow, Who, TAB_CONSOLE};
use super::pagescript::{refused_text, seed_of, ConKind, ConSeen, Console, PageScript};

/// entries of a window's history at most (the oldest go)
pub(crate) const HIST_MAX: usize = 30;
/// bytes of a page shown at most
const MAX_SHOWN: usize = 1 << 20;
/// bytes of pictures a window keeps; beyond that it starts over
const CACHE_MAX: usize = 32 << 20;
/// bytes of a style sheet at most
const SHEET_MAX: usize = 1 << 20;
const ACCEPT_SHEET: &str = "text/css, */*;q=0.1";
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
pub(crate) struct Doc {
    pub(crate) view: MdViewState,
    title: String,
    /// its URL: what images and links resolve against
    pub(crate) url: String,
    /// the status line
    status: String,
    /// its scripts (None: it has none)
    page: Option<Box<PageScript>>,
    /// the console of a page without scripts (a sheet that failed, a cookie refused)
    con: Console,
    /// what it asked for: the network log; the sheets it asked for, as it names them
    pub(crate) net: Vec<NetRow>,
    asked: BTreeSet<String>,
    /// what TLS said (https)
    tls: Option<TlsInfo>,
    /// a warning about a certificate: what to ask again, the certificate to go on with
    warn: Option<(Nav, [u8; 32])>,
    /// the document tree of the developer view, it was opened once
    pub(crate) tree: TreeState,
    pub(crate) tree_seen: bool,
}

impl Doc {
    fn new(view: MdViewState, title: String, url: String, status: String) -> Doc {
        Doc { view, title, url, status, page: None, con: Console::default(), net: Vec::new(), asked: BTreeSet::new(), tls: None, warn: None, tree: TreeState::default(), tree_seen: false }
    }

    pub(crate) fn console(&self) -> &Console {
        match &self.page {
            Some(p) => p.console(),
            None => &self.con,
        }
    }

    pub(crate) fn clear_console(&mut self) {
        match &mut self.page {
            Some(p) => p.clear_console(),
            None => self.con.clear(),
        }
    }

    fn con_push(&mut self, kind: ConKind, text: &str) {
        match &mut self.page {
            Some(p) => p.console_mut().push(kind, text),
            None => self.con.push(kind, text),
        }
    }
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
    who: Who,
    start: jfn_sim::Ns,
}

/// a picture on its way
struct Pic {
    /// as the document names it, and as it was asked for
    addr: String,
    url: String,
    doc: Rc<RefCell<Doc>>,
}

/// a style sheet on its way
struct SheetReq {
    href: String,
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
    /// the lines the console showed
    pub(crate) con_seen: ConSeen,
    /// the cookies, as of now
    pub(crate) jar: Arc<Jar>,
    /// the sheets on their way, the certificates the user went on with, the lock's card is open
    sheets: BTreeMap<u64, SheetReq>,
    accepted: Vec<[u8; 32]>,
    cert_open: bool,
    /// the developer view
    pub(crate) dev: Dev,
}

impl Browser {
    pub(crate) fn new() -> Browser {
        Browser { addr: String::from("http://"), hist: Hist::new(), load: None, pics: BTreeMap::new(), cache: BTreeMap::new(), hover: String::new(), ask: false, focus_page: false, seq: 0, scripted: BTreeMap::new(), con_seen: ConSeen::default(), jar: Arc::new(Jar::new()), sheets: BTreeMap::new(), accepted: Vec::new(), cert_open: false, dev: Dev::default() }
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
        ids.extend(self.sheets.keys().copied());
        let seq = self.seq;
        // (what lasts longer than the session stays with the device window)
        let mut jar = core::mem::take(&mut self.jar);
        Arc::make_mut(&mut jar).drop_session();
        *self = Browser::new();
        self.seq = seq;
        self.jar = jar;
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
    let u = Uri::parse(url);
    let port = u.port().map_or_else(|| String::from(if u.scheme.as_deref() == Some("https") { "443" } else { "80" }), |p| p.to_string());
    let (title, reason, hint) = match kind {
        WebErr::Refused => (t("Seite nicht erreichbar"), tf("{} lehnt die Verbindung ab: auf Port {} lauscht kein Dienst.", &[&host, &port]), t("Läuft der Webserver auf dem Gerät? Der Reiter Dienste und „ss -tln“ zeigen, was lauscht.")),
        WebErr::Timeout => (t("Seite nicht erreichbar"), tf("{} hat nicht rechtzeitig geantwortet.", &[&host]), t("Kommt das Gerät per ping an? Der Simulationsmodus zeigt, wo die Pakete bleiben.")),
        WebErr::NoName => (t("Seite nicht erreichbar"), tf("Den Namen {} kann dieses Gerät nicht auflösen.", &[&host]), t("Stimmt der Name? Ein Name braucht einen Eintrag in /etc/hosts oder einen Namensserver in /etc/resolv.conf.")),
        WebErr::NoRoute => (t("Seite nicht erreichbar"), tf("Zu {} führt von hier kein Weg.", &[&host]), t("Hat das Gerät eine Adresse und ein Gateway, und sind die Kabel gesteckt?")),
        WebErr::Broken => (t("Seite nicht erreichbar"), tf("Die Verbindung zu {} brach mitten in der Antwort ab.", &[&host]), ""),
        WebErr::Stopped => (t("Seite nicht erreichbar"), String::from(t("Die Anfrage endete, weil das Gerät oder sein Dienst angehalten hat.")), ""),
        WebErr::TooManyRedirects => (t("Zu viele Weiterleitungen"), tf("{} leitet immer weiter.", &[&host]), t("Oft zeigen zwei Seiten aufeinander; curl -L -v im Terminal zeigt die Kette.")),
        WebErr::Cert => (t("Diese Verbindung ist nicht sicher"), tf("Das Zertifikat von {} ist nicht in Ordnung.", &[&host]), ""),
        WebErr::Tls => (t("Keine sichere Verbindung"), tf("Mit {} kam keine verschlüsselte Verbindung zustande.", &[&host]), t("Spricht der Dienst auf diesem Port überhaupt TLS? Der Reiter Dienste zeigt es, der Simulationsmodus zeigt den Handshake.")),
        WebErr::BadUrl => (t("Adresse kann nicht geöffnet werden"), String::from(t("Dieser Browser öffnet http- und https-Adressen im simulierten Netz.")), ""),
    };
    let html = own_page(title, &[reason, String::from(why), String::from(hint)], url);
    (String::from(title), html)
}

/// what is wrong with a certificate, sentence by sentence, for the warning page
pub(crate) fn fault_lines(i: &TlsInfo, host: &str) -> Vec<String> {
    let mut v = Vec::new();
    let Some(c) = i.chain.first() else { return v };
    if i.faults & F_SELF_SIGNED != 0 {
        v.push(tf("Das Zertifikat ist von {} selbst unterschrieben: niemand, dem dieses Gerät vertraut, bürgt dafür.", &[&c.subject_cn]));
    }
    if i.faults & F_UNKNOWN_ISSUER != 0 {
        v.push(tf("Das Zertifikat stammt von {}: diesem Aussteller vertraut das Gerät nicht.", &[&c.issuer_cn]));
    }
    if i.faults & F_EXPIRED != 0 {
        v.push(tf("Das Zertifikat ist abgelaufen: es galt bis {}.", &[&date_text(c.not_after)]));
    }
    if i.faults & F_NOT_YET != 0 {
        v.push(tf("Das Zertifikat gilt erst ab {}.", &[&date_text(c.not_before)]));
    }
    if i.faults & F_NAME != 0 {
        v.push(tf("Das Zertifikat gilt für {}, nicht für {}.", &[&c.name_list().join(", "), host]));
    }
    if i.faults & F_BAD_SIGNATURE != 0 {
        v.push(String::from(t("Die Unterschrift des Zertifikats stimmt nicht.")));
    }
    v
}

// the warning page of a certificate with faults
fn cert_page(i: &TlsInfo, url: &str) -> (String, String) {
    let title = t("Diese Verbindung ist nicht sicher");
    let host = host_of(url);
    let mut paras = alloc::vec![tf("Der Server {} hat sich nicht ausweisen können. Wer mithört oder sich dazwischenstellt, wäre nicht zu erkennen.", &[&host])];
    paras.extend(fault_lines(i, &host));
    paras.push(String::from(t("Oben lässt sich trotzdem fortfahren: dieses Fenster merkt sich dann dieses eine Zertifikat.")));
    (String::from(title), own_page(title, &paras, url))
}

// the document of a page in HTML
fn html_doc(html: &str, title: String, url: &str, status: String) -> Doc {
    let mut view = MdViewState::new();
    view.images = MI_APP;
    view.label = String::from(t("Seite"));
    view.set_html(html);
    let title = if title.is_empty() { title_of(view.doc()) } else { title };
    Doc::new(view, title, String::from(url), status)
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
    if let Some(i) = &r.tls {
        parts.push(if i.faults != 0 { String::from(t("Zertifikat mit Fehlern")) } else { i.version.clone() });
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
            Doc::new(view, title, r.url.clone(), status)
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
    // the cookies of its hops first: what comes next reads them
    let hops: &[WebHop] = match &result {
        Ok(r) => &r.hops,
        Err(f) => &f.hops,
    };
    let refused = store_cookies(&mut s.jar, hops);
    if let Some(w) = s.scripted.remove(&id) {
        if let Some(doc) = w.upgrade() {
            let mut d = doc.borrow_mut();
            let who = d.net.iter().find(|r| r.req == id).map_or(Who::Fetch, |r| r.who);
            let start = d.net.iter().find(|r| r.req == id).map_or(0, |r| r.start);
            net_answer(&mut d.net, id, rows_of(id, who, start, &result));
            for l in &refused {
                d.con_push(ConKind::Warn, l);
            }
            if let Some(p) = d.page.as_mut() {
                p.answer(id, result.map_err(|f| tr::msg(&f.msg)));
            }
        }
        return;
    }
    if s.load.as_ref().is_some_and(|l| l.id == id) {
        let load = s.load.take();
        if let Some(l) = load {
            let rows = rows_of(id, l.who, l.start, &result);
            let (url, mut doc) = match &result {
                Ok(r) => {
                    let mut dc = doc_of(r, seed);
                    dc.tls = r.tls.clone();
                    (r.url.clone(), dc)
                }
                Err(f) => {
                    let why = tr::msg(&f.msg);
                    let (title, html) = match (&f.kind, &f.tls) {
                        (WebErr::Cert, Some(i)) => cert_page(i, &f.url),
                        _ => error_page(f.kind, &f.url, &why),
                    };
                    let status = if f.kind == WebErr::Cert { t("Zertifikat abgelehnt") } else { t("Keine Antwort") };
                    let mut dc = html_doc(&html, title, &f.url, String::from(status));
                    dc.tls = f.tls.clone();
                    if f.kind == WebErr::Cert {
                        let fp = f.tls.as_ref().and_then(|i| i.chain.first()).map(|c| c.fingerprint());
                        dc.warn = fp.map(|x| (l.nav.clone(), x));
                    }
                    (f.url.clone(), dc)
                }
            };
            doc.net = rows;
            for x in &refused {
                doc.con_push(ConKind::Warn, x);
            }
            s.cert_open = false;
            arrive(s, l.nav, l.replace, url, doc);
        }
        return;
    }
    if let Some(sh) = s.sheets.remove(&id) {
        sheet_came(s, id, sh, &result, &refused);
        return;
    }
    let Some(p) = s.pics.remove(&id) else { return };
    {
        let mut d = p.doc.borrow_mut();
        let start = d.net.iter().find(|r| r.req == id).map_or(0, |r| r.start);
        net_answer(&mut d.net, id, rows_of(id, Who::Image, start, &result));
        for l in &refused {
            d.con_push(ConKind::Warn, l);
        }
    }
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

/// the Set-Cookie headers of a request's hops into the jar, in their order, each at the clock of its
/// answer -> what it refused, as console lines
pub(crate) fn store_cookies(jar: &mut Arc<Jar>, hops: &[WebHop]) -> Vec<String> {
    let mut out = Vec::new();
    for h in hops {
        let u = Uri::parse(&h.url);
        let Some(host) = u.host() else { continue };
        let path = if u.path.is_empty() { "/" } else { u.path.as_str() };
        let secure = u.scheme.as_deref() == Some("https");
        for (k, v) in &h.headers.0 {
            if !k.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            let o = Origin { host, path, secure, http: true };
            if let Err(e) = Arc::make_mut(jar).store(v, o, h.clock) {
                let name = v.split(['=', ';']).next().unwrap_or("").trim();
                out.push(tf("Cookie {} von {} nicht gespeichert: {}", &[name, host, refused_text(e)]));
            }
        }
    }
    out
}

/// what a sheet's answer gives: its text, or the console's line why not
pub(crate) fn sheet_text(result: &Result<WebResp, WebFail>, url: &str) -> Result<String, String> {
    match result {
        Ok(r) if r.status < 200 || r.status >= 300 => Err(tf("{}: das Stylesheet kam nicht: {} {}", &[url, &r.status.to_string(), &r.reason])),
        Ok(r) if r.body.len() > SHEET_MAX => Err(tf("{}: das Stylesheet ist zu groß ({} Bytes)", &[url, &r.body.len().to_string()])),
        Ok(r) => {
            let ctype = r.headers.get("content-type").unwrap_or("");
            let kind = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            if !kind.is_empty() && kind != "text/css" {
                return Err(tf("{}: das ist kein Stylesheet: Typ {}", &[url, &kind]));
            }
            Ok(text_of(ctype, &r.body).0)
        }
        Err(f) => Err(tf("{}: das Stylesheet kam nicht: {}", &[url, &tr::msg(&f.msg)])),
    }
}

// a style sheet's answer: its text into the document, or a line of the console
fn sheet_came(s: &mut Browser, id: u64, sh: SheetReq, result: &Result<WebResp, WebFail>, refused: &[String]) {
    let mut d = sh.doc.borrow_mut();
    let start = d.net.iter().find(|r| r.req == id).map_or(0, |r| r.start);
    net_answer(&mut d.net, id, rows_of(id, Who::Sheet, start, result));
    for l in refused {
        d.con_push(ConKind::Warn, l);
    }
    match sheet_text(result, &sh.url) {
        Ok(text) => {
            d.view.doc_mut().set_sheet(&sh.href, Some(&text));
            if s.cache.values().map(|x| x.len()).sum::<usize>() + text.len() > CACHE_MAX {
                s.cache.clear();
            }
            s.cache.insert(sh.url.clone(), Rc::new(text.into_bytes()));
        }
        Err(l) => d.con_push(ConKind::Error, &l),
    }
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

// what a request carries of the window: its cookies, who asks (the site of a page; "" the user), a
// navigation or not, the certificates the user went on with
fn stamp(s: &Browser, req: &mut WebReq, page: Option<&str>, top: bool) {
    req.jar = Some(s.jar.clone());
    req.site = page.map_or(String::new(), |p| Uri::parse(p).host().map(site_of).unwrap_or_default());
    req.top = top;
    req.accept = s.accepted.clone();
}

// a request for a page, or at once a page of ours when it cannot be asked for; who: the address line
// and a reload come from the user, a link, a form and a script from the page shown
fn go(n: &mut Net, k: usize, dev: u32, nav: Nav, replace: bool, who: Who) {
    drop_load(n, k, dev);
    let now = n.doc.sim.engine().now();
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.web;
    s.ask = false;
    let u = Uri::parse(&nav.url);
    let local = match u.scheme.as_deref() {
        Some("http") | Some("https") => None,
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
    let mut req = WebReq { id, method: nav.method.clone(), url: nav.url.clone(), headers, body: nav.body.clone(), ..WebReq::default() };
    let page = s.hist.cur().map(|e| e.url.clone()).filter(|_| !matches!(who, Who::Address | Who::Reload));
    stamp(s, &mut req, page.as_deref(), true);
    s.load = Some(Load { id, nav, replace, who, start: now });
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
fn follow(n: &mut Net, k: usize, dev: u32, target: &str, who: Who) {
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
        _ => go(n, k, dev, Nav::get(&u.text()), false, who),
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
        go(n, k, dev, Nav::get(&url), true, Who::Reload);
    } else {
        go(n, k, dev, Nav::get(&url), false, Who::Address);
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
    go(n, k, dev, nav, true, Who::Reload);
}

// what the document wants: the pictures it waits for
fn fetch_pictures(n: &mut Net, k: usize, dev: u32, doc: &Rc<RefCell<Doc>>, wanted: Vec<String>) {
    let now = n.doc.sim.engine().now();
    for addr in wanted {
        let page = doc.borrow().url.clone();
        let abs = Uri::parse(&page).join(&addr).encoded();
        if abs.scheme.as_deref() == Some("data") {
            let bytes = data_url(&addr);
            doc.borrow_mut().view.set_image(&addr, bytes.as_deref());
            continue;
        }
        if !matches!(abs.scheme.as_deref(), Some("http") | Some("https")) {
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
        let mut req = WebReq { id, method: String::from("GET"), url: url.clone(), headers, body: Vec::new(), ..WebReq::default() };
        stamp(s, &mut req, Some(&page), false);
        net_add(&mut doc.borrow_mut().net, pending_row(&req, Who::Image, now));
        s.pics.insert(id, Pic { addr: addr.clone(), url, doc: doc.clone() });
        if n.doc.sim.send(Cmd::Apply(Action::WebFetch { dev, req })).is_err() {
            if let Some(w) = n.devwin.get(k) {
                w.desk.web.pics.remove(&id);
            }
            net_stopped(&mut doc.borrow_mut().net, id);
            doc.borrow_mut().view.set_image(&addr, None);
        }
    }
}

// the style sheets a document names that it has not asked for yet
fn fetch_sheets(n: &mut Net, k: usize, dev: u32, doc: &Rc<RefCell<Doc>>) {
    let new: Vec<String> = {
        let d = doc.borrow();
        d.view.doc().sheet_links().iter().filter(|l| !d.asked.contains(*l)).cloned().collect()
    };
    if new.is_empty() {
        return;
    }
    let now = n.doc.sim.engine().now();
    for href in new {
        doc.borrow_mut().asked.insert(href.clone());
        let page = doc.borrow().url.clone();
        let abs = Uri::parse(&page).join(&href).encoded();
        if abs.scheme.as_deref() == Some("data") {
            let bytes = data_url(&href).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes).into_owned();
            doc.borrow_mut().view.doc_mut().set_sheet(&href, Some(&text));
            continue;
        }
        if !matches!(abs.scheme.as_deref(), Some("http") | Some("https")) {
            let l = tf("{}: das Stylesheet kam nicht: nur http und https gehen", &[&href]);
            doc.borrow_mut().con_push(ConKind::Error, &l);
            continue;
        }
        let url = Uri { fragment: None, ..abs }.text();
        let Some(w) = n.devwin.get(k) else { return };
        let s = &mut w.desk.web;
        if let Some(b) = s.cache.get(&url).cloned() {
            let text = String::from_utf8_lossy(&b).into_owned();
            doc.borrow_mut().view.doc_mut().set_sheet(&href, Some(&text));
            continue;
        }
        s.seq += 1;
        let id = s.seq;
        let mut headers = Headers::new();
        headers.add("Accept", ACCEPT_SHEET);
        let mut req = WebReq { id, method: String::from("GET"), url: url.clone(), headers, body: Vec::new(), ..WebReq::default() };
        stamp(s, &mut req, Some(&page), false);
        net_add(&mut doc.borrow_mut().net, pending_row(&req, Who::Sheet, now));
        s.sheets.insert(id, SheetReq { href, url: url.clone(), doc: doc.clone() });
        if let Err(e) = n.doc.sim.send(Cmd::Apply(Action::WebFetch { dev, req })) {
            let why = tr::msg(&e.msg());
            if let Some(w) = n.devwin.get(k) {
                w.desk.web.sheets.remove(&id);
            }
            net_stopped(&mut doc.borrow_mut().net, id);
            let l = tf("{}: das Stylesheet kam nicht: {}", &[&url, &why]);
            doc.borrow_mut().con_push(ConKind::Error, &l);
        }
    }
}

// Anhalten: the page and the pictures on their way are given up
fn stop(n: &mut Net, k: usize, dev: u32) {
    drop_load(n, k, dev);
    let Some(w) = n.devwin.get(k) else { return };
    let pics = core::mem::take(&mut w.desk.web.pics);
    for (id, p) in pics {
        net_stopped(&mut p.doc.borrow_mut().net, id);
        p.doc.borrow_mut().view.set_image(&p.addr, None);
        let _ = n.doc.sim.send(Cmd::Apply(Action::WebCancel { dev, id }));
    }
    let Some(w) = n.devwin.get(k) else { return };
    let sheets = core::mem::take(&mut w.desk.web.sheets);
    let scripted = core::mem::take(&mut w.desk.web.scripted);
    for (id, sh) in sheets {
        net_stopped(&mut sh.doc.borrow_mut().net, id);
        let l = tf("{}: das Stylesheet kam nicht: {}", &[&sh.url, t("Angehalten")]);
        sh.doc.borrow_mut().con_push(ConKind::Warn, &l);
        let _ = n.doc.sim.send(Cmd::Apply(Action::WebCancel { dev, id }));
    }
    // fetch and src: their callbacks hear why, the page goes on
    for (id, weak) in scripted {
        if let Some(doc) = weak.upgrade() {
            let mut d = doc.borrow_mut();
            net_stopped(&mut d.net, id);
            if let Some(p) = d.page.as_mut() {
                p.answer(id, Err(String::from(t("Angehalten"))));
            }
        }
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
            net_stopped(&mut p.doc.borrow_mut().net, id);
            p.doc.borrow_mut().view.set_image(&p.addr, None);
        }
    }
    let gone: Vec<u64> = s.sheets.keys().copied().filter(|id| !active.contains(id)).collect();
    for id in gone {
        if let Some(sh) = s.sheets.remove(&id) {
            net_stopped(&mut sh.doc.borrow_mut().net, id);
        }
    }
    let gone: Vec<u64> = s.scripted.keys().copied().filter(|id| !active.contains(id)).collect();
    for id in gone {
        if let Some(doc) = s.scripted.remove(&id).and_then(|w| w.upgrade()) {
            net_stopped(&mut doc.borrow_mut().net, id);
            if let Some(p) = doc.borrow_mut().page.as_mut() {
                p.answer(id, Err(String::from(t("Die Anfrage endete, weil das Gerät oder sein Dienst angehalten hat."))));
            }
        }
    }
}

// the scripts of the document shown: what is due now, and a frame for what comes next
fn page_pump(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let now = n.doc.sim.engine().now();
    let clock = clock_of(n, dev);
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
            p.jar_in(&mut s.jar, clock);
            p.pump(&mut d.view, now, &mut s.seq);
            p.jar_out(&mut s.jar);
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
    let now = n.doc.sim.engine().now();
    for mut req in fx.fetch {
        let id = req.id;
        let page = doc.borrow().url.clone();
        let who = if doc.borrow().page.as_ref().and_then(|p| p.src_request()) == Some(id) { Who::Src } else { Who::Fetch };
        if let Some(w) = n.devwin.get(k) {
            stamp(&w.desk.web, &mut req, Some(&page), false);
            w.desk.web.scripted.insert(id, Rc::downgrade(doc));
        }
        net_add(&mut doc.borrow_mut().net, pending_row(&req, who, now));
        if let Err(e) = n.doc.sim.send(Cmd::Apply(Action::WebFetch { dev, req })) {
            let why = tr::msg(&e.msg());
            if let Some(w) = n.devwin.get(k) {
                w.desk.web.scripted.remove(&id);
            }
            net_stopped(&mut doc.borrow_mut().net, id);
            if let Some(p) = doc.borrow_mut().page.as_mut() {
                p.answer(id, Err(why));
            }
            ui.repass();
        }
    }
    if let Some(target) = fx.nav {
        follow(n, k, dev, &target, Who::Script);
        ui.repass();
    } else if fx.reload {
        reload(n, k, dev, false);
        ui.repass();
    }
}

// the device clock's seconds (what cookies expire by)
fn clock_of(n: &Net, dev: u32) -> i64 {
    n.doc.sim.engine().clock(dev).unwrap_or(jfn_sim::EPOCH_UNIX_NS).div_euclid(1_000_000_000)
}

// ---------------------------------------------------------------- the window

// the card of a page's certificate
fn cert_card(ui: &mut Ui, i: &TlsInfo, url: &str) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 4);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.icon(if i.faults != 0 { IC_WARNING } else { IC_LOCK }, 16);
    ui.text(TS_BODY_STRONG, if i.faults != 0 { t("Verschlüsselt, aber das Zertifikat hat Fehler") } else { t("Verschlüsselte Verbindung") });
    ui.box_end();
    let host = host_of(url);
    let mut rows: Vec<(&str, String)> = alloc::vec![(t("Verbindung"), format!("{}, {}", i.version, i.suite))];
    if let Some(c) = i.chain.first() {
        rows.push((t("Ausgestellt für"), if c.subject_org.is_empty() { c.subject_cn.clone() } else { format!("{} ({})", c.subject_cn, c.subject_org) }));
        rows.push((t("Ausgestellt von"), if c.issuer_org.is_empty() { c.issuer_cn.clone() } else { format!("{} ({})", c.issuer_cn, c.issuer_org) }));
        rows.push((t("Gültig ab"), date_text(c.not_before)));
        rows.push((t("Gültig bis"), date_text(c.not_after)));
        rows.push((t("Namen"), c.name_list().join(", ")));
        rows.push((t("Fingerabdruck (SHA-256)"), c.fingerprint_text()));
    }
    ui.gridbox_begin(0, &[0, -1], 0, 4);
    for (k, v) in &rows {
        ui.text(TS_CAPTION | TX_SECONDARY, k);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, v);
    }
    ui.gridbox_end();
    for l in fault_lines(i, &host) {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &l);
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, t("Ein Zertifikat aus dem simulierten Netz: seine Schlüssel kommen aus dem Startwert des Projekts und schützen nichts Echtes."));
    ui.box_end();
}

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
    /// the warning's Trotzdem fortfahren
    GoOn,
}

/// the Webbrowser of device window k
pub(crate) fn program(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    lost_requests(n, k, dev);
    page_pump(n, ui, k, dev);
    let mode = n.doc.sim.mode();
    let now = n.doc.sim.engine().now();
    let clock = clock_of(n, dev);
    let page_id = ID_BROWSER0 + 0x100 * k as u32;
    let con_id = ID_PAGE0 + 0x80 * k as u32;
    let dev_id = ID_DEV0 + 0x80 * k as u32;
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
        } else if (s.load.is_some() || !s.pics.is_empty() || !s.sheets.is_empty() || !s.scripted.is_empty()) && ui.capture_shortcut(VK_ESCAPE, 0) {
            act = Some(Act::Stop);
        } else if ui.capture_shortcut(b'L' as u32, MOD_CTRL) {
            ui.focus(id);
            let text = s.addr.clone();
            ui.textbox_select(id, &text, 0, text.len());
        } else if scripts && ui.capture_shortcut(b'J' as u32, MOD_CTRL | MOD_SHIFT) {
            toggle_console(s);
            ui.repass();
        } else if ui.capture_shortcut(VK_F12, 0) {
            s.dev.open = !s.dev.open;
            ui.repass();
        }
    }
    let loading = s.load.is_some();
    // what the button stops: all that is on its way
    let busy = loading || !s.pics.is_empty() || !s.sheets.is_empty() || !s.scripted.is_empty();
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
    ui.next_icon(if busy { IC_STOP } else { IC_REFRESH });
    ui.next_flags(flag(busy || s.hist.cur().is_some()));
    if ui.button(id + 5, if busy { t("Anhalten") } else { t("Neu laden") }) & EV_ACT != 0 {
        act = Some(if busy { Act::Stop } else { Act::Reload });
    }
    ui.tooltip(if busy { t("Anhalten (Esc)") } else { t("Neu laden (F5)") });
    // the lock of a page that came over TLS
    let tls = s.hist.cur().and_then(|e| e.doc.borrow().tls.clone());
    if let Some(i) = &tls {
        ui.next_icon(if i.faults != 0 { IC_WARNING } else { IC_LOCK });
        ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
        if ui.button(dev_id + 1, t("Zertifikat")) & EV_ACT != 0 {
            s.cert_open = !s.cert_open;
            ui.repass();
        }
        ui.tooltip(if i.faults != 0 { t("Verschlüsselt, aber das Zertifikat hat Fehler") } else { t("Verschlüsselte Verbindung: das Zertifikat zeigen") });
    }
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
        let mut on = (s.dev.open && s.dev.tab == TAB_CONSOLE) as i32;
        ui.next_icon(crate::workspace::icons::JN_TERMINAL);
        ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
        if ui.toggle_button(con_id, t("Konsole"), &mut on) & EV_ACT != 0 {
            toggle_console(s);
            ui.repass();
        }
        ui.tooltip(t("Konsole der Seitenskripte (Strg+Umschalt+J)"));
    }
    let mut on = s.dev.open as i32;
    ui.next_icon(crate::workspace::icons::JN_APP_IDE);
    ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
    if ui.toggle_button(dev_id, t("Entwickleransicht"), &mut on) & EV_ACT != 0 {
        s.dev.open = on != 0;
        ui.repass();
    }
    ui.tooltip(t("Entwickleransicht: Konsole, Netzwerk, Elemente, Cookies (F12)"));
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
    // a certificate with faults: the question above its page
    let warned = s.hist.cur().is_some_and(|e| e.doc.borrow().warn.is_some());
    if warned {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_CARD | pad(6), 8);
        ui.icon(IC_WARNING, 16);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, t("Das Zertifikat dieses Servers hat Fehler. Nur fortfahren, wenn klar ist, warum."));
        if ui.button(dev_id + 2, t("Trotzdem fortfahren")) & EV_ACT != 0 {
            act = Some(Act::GoOn);
        }
        if s.hist.can_back() {
            if ui.accent_button(dev_id + 3, t("Zurück zur sicheren Seite")) & EV_ACT != 0 {
                act = Some(Act::Back);
            }
        }
        ui.box_end();
    }
    if s.cert_open {
        if let Some(i) = &tls {
            let url = s.hist.cur().map(|e| e.url.clone()).unwrap_or_default();
            cert_card(ui, i, &url);
        }
    }
    // the page
    let doc = s.hist.cur().map(|e| e.doc.clone());
    let mut wanted = Vec::new();
    // the developer view: the page and it share the room, with a splitter
    let dev_open = s.dev.open && doc.is_some();
    if dev_open {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_FIXED, 0);
        let h = ui.px2dip(ui.box_rect()[3]);
        // (until the user drags it the pane has a share of the room; it never gets squeezed out)
        let pos = if s.dev.pos == 0 { h * 40 / 100 } else { s.dev.pos.min(h - 180) };
        ui.next_size(-1, -1);
        ui.split_begin(dev_id + 5, pos.max(80), true);
    }
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
                p.jar_in(&mut s.jar, clock);
                let (link, submit) = p.events(&mut dd.view, ev.clone(), now, &mut s.seq);
                p.jar_out(&mut s.jar);
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
    // the developer view below it
    if dev_open {
        ui.split_next();
        let mut close = false;
        if let Some(d) = &doc {
            close = devtools::pane(ui, dev_id, con_id, s, d, clock);
        }
        ui.split_end(&mut s.dev.pos);
        ui.box_end();
        if close {
            s.dev.open = false;
            ui.repass();
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
        let errors = d.borrow().console().errors();
        if errors > 0 && !s.dev.open {
            st = if scripts { tf("{} · Skriptfehler: {} (Konsole)", &[&st, &errors.to_string()]) } else { tf("{} · Fehler: {} (Entwickleransicht, F12)", &[&st, &errors.to_string()]) };
        }
        match waiting {
            0 => st,
            1 => tf("{} · ein Bild lädt noch …", &[&st]),
            w => tf("{} · {} Bilder laden noch …", &[&st, &w.to_string()]),
        }
    } else {
        String::new()
    };
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &line);
    // what the widgets and keys asked for, after them
    if let Some(d) = &doc {
        if !wanted.is_empty() {
            fetch_pictures(n, k, dev, d, wanted);
        }
        fetch_sheets(n, k, dev, d);
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
        Act::Link(l) => follow(n, k, dev, &l, Who::Link),
        Act::Submit(f) => {
            s.focus_page = true;
            let page = s.hist.cur().map(|e| e.url.clone()).unwrap_or_default();
            go(n, k, dev, form_nav(&page, &f), false, Who::Form);
        }
        Act::GoOn => {
            let warn = s.hist.cur().and_then(|e| e.doc.borrow().warn.clone());
            if let Some((nav, fp)) = warn {
                if !s.accepted.contains(&fp) {
                    s.accepted.push(fp);
                }
                s.focus_page = true;
                go(n, k, dev, nav, true, Who::Address);
            }
        }
    }
    ui.repass();
}

// the console's tab: open on it, or shut when it shows
fn toggle_console(s: &mut Browser) {
    if s.dev.open && s.dev.tab == TAB_CONSOLE {
        s.dev.open = false;
    } else {
        s.dev.open = true;
        s.dev.tab = TAB_CONSOLE;
    }
}
