// devtools -- the Webbrowser's developer view (B4): a pane below the page with the console of B3, the
// requests of the page, its document tree and its cookies.  It is the window's state, no undo step and
// no project change; the requests go through the engine as all of them do, what is shown here comes from
// their reports
//
//   Dev::default()                the pane's state (Browser::dev): open, the tab, the sizes, the choices
//   Who, NetRow                   who asked (Who::text()), a row of the network log
//   pending_row(req, who, now), rows_of(id, who, start, result)   a request on its way, what its report
//                                 says: a row for every hop, the last one with the body
//   tree_rows(doc, tree, max) -> Vec<TreeLine>   the visible lines of a document's tree (live: every
//                                 frame from the document as scripts leave it)
//   element_lines(doc, node) -> (attributes, styles)   what the Elemente tab shows of an element
//   body_text(row) -> String      what the Netzwerk tab shows of a body (text, or the first bytes in hex)
//   cookie_rows(jar, url, clock)  the cookies of the page as text for the table
//   pane(ui, base, s, doc, clock) the pane of a window: base the block of ids (below)
//
// A row's key is the request's number and the hop: (id << 8) | hop.  Bodies are kept up to BODY_MAX per
// row and rows up to ROWS_MAX per document (the oldest go); a document's log goes with the document.
//
// Ids: base = ID_DEV0 + 0x80 k: + 0 its button, + 1 the lock, + 2 / + 3 the warning's buttons, + 4 Schließen,
// + 5 the outer split, + 6 the tabs, + 0x10 the inner split, + 0x18 the details, + 0x20 Cookie löschen,
// + 0x21 Alle löschen, + 0x28 the table of requests, + 0x2C the table of cookies, + 0x30 the tree (its
// rows after it; one tab at a time).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::webc::{TlsInfo, WebHop};
use jfn_sim::{Ns, WebFail, WebReq, WebResp};
use netpkt::cookie::{Asks, Cookie, Jar};
use netpkt::http::Headers;
use netpkt::uri::Uri;
use ui::*;

use crate::app::tr::{self, t, tf};

use super::browser::{Browser, Doc};

/// bytes of a body a row keeps
const BODY_MAX: usize = 256 << 10;
/// bytes of a body the pane shows
const SHOWN_MAX: usize = 32 << 10;
/// rows of a document's log
const ROWS_MAX: usize = 500;
/// lines of the document tree
const LINES_MAX: usize = 3000;

pub(crate) const TAB_CONSOLE: i32 = 0;
pub(crate) const TAB_NET: i32 = 1;
pub(crate) const TAB_ELEMENTS: i32 = 2;
pub(crate) const TAB_COOKIES: i32 = 3;

/// who asked for a request
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Who {
    #[default]
    Address,
    Reload,
    Link,
    Form,
    Script,
    Image,
    Sheet,
    Src,
    Fetch,
}

impl Who {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Who::Address => t("Adresszeile"),
            Who::Reload => t("Neu laden"),
            Who::Link => t("Link"),
            Who::Form => t("Formular"),
            Who::Script => t("Skript"),
            Who::Image => "<img>",
            Who::Sheet => "<link>",
            Who::Src => "<script src>",
            Who::Fetch => "fetch",
        }
    }
}

/// a row of the network log
#[derive(Clone, Debug, Default)]
pub(crate) struct NetRow {
    pub key: u64,
    pub req: u64,
    pub who: Who,
    pub method: String,
    pub url: String,
    /// on its way
    pub pending: bool,
    /// 0: no answer
    pub status: u16,
    pub reason: String,
    /// why no answer came
    pub failed: String,
    pub ctype: String,
    pub size: usize,
    pub start: Ns,
    pub took: Ns,
    pub sent: Headers,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub cut: bool,
    pub tls: Option<TlsInfo>,
}

impl NetRow {
    fn status_text(&self) -> String {
        if self.pending {
            String::from("…")
        } else if self.status > 0 {
            format!("{} {}", self.status, self.reason)
        } else {
            String::from(t("Fehler"))
        }
    }
}

/// a request on its way
pub(crate) fn pending_row(r: &WebReq, who: Who, now: Ns) -> NetRow {
    NetRow { key: r.id << 8, req: r.id, who, method: r.method.clone(), url: r.url.clone(), pending: true, start: now, sent: r.headers.clone(), ..NetRow::default() }
}

fn hop_row(id: u64, i: usize, who: Who, start: Ns, h: &WebHop) -> NetRow {
    let ctype = h.headers.get("content-type").unwrap_or("");
    NetRow { key: id << 8 | i as u64, req: id, who, method: h.method.clone(), url: h.url.clone(), status: h.status, reason: h.reason.clone(), ctype: String::from(ctype.split(';').next().unwrap_or("").trim()), start, took: h.took, sent: h.sent.clone(), headers: h.headers.clone(), tls: h.tls.clone(), ..NetRow::default() }
}

/// what a report says: a row for every hop (a redirect is a row of its own)
pub(crate) fn rows_of(id: u64, who: Who, start: Ns, result: &Result<WebResp, WebFail>) -> Vec<NetRow> {
    let mut rows = Vec::new();
    let mut at = start;
    let hops: &[WebHop] = match result {
        Ok(r) => &r.hops,
        Err(f) => &f.hops,
    };
    for (i, h) in hops.iter().enumerate() {
        let row = hop_row(id, i, who, at, h);
        at += h.took;
        rows.push(row);
    }
    match result {
        Ok(r) => {
            if let Some(l) = rows.last_mut() {
                l.size = r.body.len();
                l.cut = r.body.len() > BODY_MAX;
                l.body = Vec::from(&r.body[..r.body.len().min(BODY_MAX)]);
            }
        }
        Err(f) => {
            if rows.is_empty() {
                rows.push(NetRow { key: id << 8, req: id, who, url: f.url.clone(), start, ..NetRow::default() });
            }
            if let Some(l) = rows.last_mut() {
                l.failed = tr::msg(&f.msg);
            }
        }
    }
    rows
}

/// a row into a document's log
pub(crate) fn net_add(log: &mut Vec<NetRow>, row: NetRow) {
    if log.len() >= ROWS_MAX {
        log.remove(0);
    }
    log.push(row);
}

/// a request's rows in the place of its pending one
pub(crate) fn net_answer(log: &mut Vec<NetRow>, id: u64, rows: Vec<NetRow>) {
    let at = log.iter().position(|r| r.req == id).unwrap_or(log.len());
    log.retain(|r| r.req != id);
    let at = at.min(log.len());
    for (i, r) in rows.into_iter().enumerate() {
        log.insert(at + i, r);
    }
    while log.len() > ROWS_MAX {
        log.remove(0);
    }
}

/// a request given up
pub(crate) fn net_stopped(log: &mut [NetRow], id: u64) {
    for r in log.iter_mut().filter(|r| r.req == id && r.pending) {
        r.pending = false;
        r.failed = String::from(t("Angehalten"));
    }
}

// ---------------------------------------------------------------- the pane's parts

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return String::from(s);
    }
    let mut o: String = s.chars().take(n).collect();
    o.push('…');
    o
}

/// a line of the document tree
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeLine {
    pub key: u64,
    pub depth: u16,
    pub children: bool,
    pub label: String,
}

fn shows(d: &MdDoc, n: MdNode) -> bool {
    !d.is_text(n) || !d.text_of(n).trim().is_empty()
}

fn label_of(d: &MdDoc, n: MdNode) -> String {
    if d.is_text(n) {
        let x = d.text_of(n);
        return format!("\"{}\"", clip(x.trim(), 60));
    }
    let mut s = format!("<{}", d.tag(n));
    for (k, v) in d.attrs(n).iter().take(6) {
        if v.is_empty() {
            s.push_str(&format!(" {}", k));
        } else {
            s.push_str(&format!(" {}=\"{}\"", k, clip(v, 40)));
        }
    }
    if d.attrs(n).len() > 6 {
        s.push_str(" …");
    }
    s.push('>');
    s
}

/// the visible lines of a document's tree: below what is open in `tree`, at most `max`
pub(crate) fn tree_rows(d: &MdDoc, tree: &TreeState, max: usize) -> Vec<TreeLine> {
    let mut out = Vec::new();
    let root = d.root();
    if root.is_none() {
        return out;
    }
    let kids = |n: MdNode| -> Vec<MdNode> { d.children(n).into_iter().filter(|c| shows(d, *c)).collect() };
    let mut stack: Vec<(MdNode, u16)> = kids(root).into_iter().rev().map(|c| (c, 0)).collect();
    while let Some((n, depth)) = stack.pop() {
        if out.len() >= max {
            break;
        }
        let below = kids(n);
        let open = tree.expanded.contains(&n.raw());
        out.push(TreeLine { key: n.raw(), depth, children: !below.is_empty(), label: label_of(d, n) });
        if open {
            for c in below.into_iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
    out
}

/// what the Elemente tab shows of an element: its attributes, the declarations the cascade gives it
pub(crate) fn element_lines(d: &MdDoc, n: MdNode) -> (Vec<String>, Vec<String>) {
    let attrs = d.attrs(n).iter().map(|(k, v)| format!("{} = \"{}\"", k, v)).collect();
    let style = d.style_of(n);
    let styles = style.split(';').map(str::trim).filter(|x| !x.is_empty()).map(|x| x.replace(':', ": ")).collect();
    (attrs, styles)
}

/// what the Netzwerk tab shows of a body
pub(crate) fn body_text(r: &NetRow) -> String {
    if r.body.is_empty() {
        return String::new();
    }
    let ty = r.ctype.as_str();
    let texty = ty.starts_with("text/") || ty.ends_with("json") || ty.ends_with("xml") || ty.ends_with("javascript") || core::str::from_utf8(&r.body).is_ok();
    if !texty || ty.starts_with("image/") && ty != "image/svg+xml" {
        let hex: Vec<String> = r.body.iter().take(64).map(|b| format!("{:02x}", b)).collect();
        return tf("{} Bytes, zu Beginn: {}", &[&r.size.to_string(), &hex.join(" ")]);
    }
    let cut = r.body.len() > SHOWN_MAX;
    let mut s = String::from_utf8_lossy(&r.body[..r.body.len().min(SHOWN_MAX)]).into_owned();
    if cut || r.cut {
        s.push_str("\n…");
    }
    s
}

/// a date as the pane and the certificate's card write it
pub(crate) fn date_text(secs: i64) -> String {
    let c = datetime::Civil::from_unix(secs);
    format!("{:04}-{:02}-{:02} {:02}:{:02} UTC", c.year, c.month, c.day, c.hour, c.minute)
}

/// the cookies of the page shown (what a request to it would carry, HttpOnly ones too)
pub(crate) fn page_cookies<'a>(jar: &'a Jar, url: &str, clock: i64) -> Vec<&'a Cookie> {
    let u = Uri::parse(url);
    let Some(host) = u.host() else { return Vec::new() };
    let path = if u.path.is_empty() { "/" } else { u.path.as_str() };
    jar.matching(&Asks { host, path, secure: u.scheme.as_deref() == Some("https"), http: true, site: None, top: true, safe: true }, clock)
}

fn expiry_text(c: &Cookie) -> String {
    match c.expires {
        Some(e) => date_text(e),
        None => String::from(t("Sitzung")),
    }
}

fn flags_text(c: &Cookie) -> String {
    let mut v: Vec<&str> = Vec::new();
    if c.secure {
        v.push("Secure");
    }
    if c.http_only {
        v.push("HttpOnly");
    }
    if !c.same_site.name().is_empty() {
        v.push(c.same_site.name());
    }
    v.join(", ")
}

/// the pane's state
#[derive(Default)]
pub(crate) struct Dev {
    pub open: bool,
    pub tab: i32,
    /// the page's height over the pane as the user dragged it (0: not dragged)
    pub pos: i32,
    /// the list's size beside or above the details
    pub inner: i32,
    pub net: GridState,
    pub cookies: GridState,
}

// the head of a message as lines
fn head_lines(ui: &mut Ui, h: &Headers) {
    for (k, v) in &h.0 {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &format!("{}: {}", k, v));
    }
}

fn caption(ui: &mut Ui, s: &str) {
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG, s);
}

// the details of a request
fn net_details(ui: &mut Ui, id: u32, r: &NetRow) {
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | pad(4), 3);
    caption(ui, t("Anfrage"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_WRAP, &format!("{} {}", r.method, r.url));
    head_lines(ui, &r.sent);
    caption(ui, t("Antwort"));
    ui.next_size(-1, 0);
    if r.pending {
        ui.text(TS_CAPTION | TX_WRAP, t("Noch keine Antwort."));
    } else if r.status > 0 {
        ui.text(TS_CAPTION | TX_WRAP, &format!("{} {}", r.status, r.reason));
        head_lines(ui, &r.headers);
    } else {
        ui.text(TS_CAPTION | TX_WRAP, &r.failed);
    }
    if let Some(c) = r.tls.as_ref() {
        caption(ui, "TLS");
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &format!("{}, {}", c.version, c.suite));
        if let Some(l) = c.chain.first() {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_WRAP, &tf("Zertifikat für {}, ausgestellt von {}", &[&l.subject_cn, &l.issuer_cn]));
        }
    }
    let body = body_text(r);
    if !body.is_empty() {
        caption(ui, &tf("Inhalt ({} Bytes)", &[&r.size.to_string()]));
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &body);
    }
    ui.box_end();
    ui.scroll_end();
}

// the table and the details
fn net_tab(ui: &mut Ui, base: u32, dev: &mut Dev, rows: &[NetRow]) {
    let keys: Vec<u64> = rows.iter().map(|r| r.key).collect();
    dev.net.retain(&keys);
    let chosen = dev.net.cur.and_then(|k| rows.iter().find(|r| r.key == k));
    if rows.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 4);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Diese Seite hat noch keine Anfragen."));
        ui.box_end();
        return;
    }
    let cols = [
        GridCol::new(t("Methode"), 72).flags(GC_NOSORT),
        GridCol::new(t("Adresse"), 180).flags(GC_NOSORT | GC_FILL),
        GridCol::new(t("Status"), 96).flags(GC_NOSORT),
        GridCol::new(t("Typ"), 92).flags(GC_NOSORT),
        GridCol::new(t("Größe"), 62).flags(GC_NOSORT | GC_RIGHT),
        GridCol::new(t("Zeit"), 62).flags(GC_NOSORT | GC_RIGHT),
        GridCol::new(t("Wer fragte"), 86).flags(GC_NOSORT),
    ];
    let rect = ui.box_rect();
    let wide = ui.px2dip(rect[2]) >= 760;
    let split = chosen.is_some();
    if split {
        let total = ui.px2dip(if wide { rect[2] } else { rect[3] });
        if dev.inner == 0 && total > 0 {
            dev.inner = total * if wide { 58 } else { 45 } / 100;
        }
        ui.next_size(-1, -1);
        ui.split_begin(base + 0x10, dev.inner.max(80), !wide);
    }
    ui.next_size(-1, -1);
    let (first, end) = ui.grid_begin(base + 0x28, &mut dev.net, &cols, &keys);
    for i in first..end {
        let r = &rows[i as usize];
        ui.grid_row(&mut dev.net, i);
        ui.grid_text(&dev.net, 0, &r.method);
        ui.grid_text(&dev.net, 1, &r.url);
        ui.grid_text(&dev.net, 2, &r.status_text());
        ui.grid_text(&dev.net, 3, &r.ctype);
        ui.grid_text(&dev.net, 4, &if r.status > 0 { r.size.to_string() } else { String::new() });
        ui.grid_text(&dev.net, 5, &if r.pending { String::new() } else { tr::msg(&jfn_model::msg::dur(r.took as i64)) });
        ui.grid_text(&dev.net, 6, r.who.text());
    }
    let ev = ui.grid_end(&mut dev.net);
    if ev.ev & EV_ACT != 0 {
        ui.repass();
    }
    if split {
        ui.split_next();
        if let Some(r) = chosen {
            net_details(ui, base + 0x18, r);
        }
        ui.split_end(&mut dev.inner);
    }
}

// the tree and what the chosen element has
fn elements_tab(ui: &mut Ui, base: u32, dev: &mut Dev, doc: &mut Doc) {
    let d = doc.view.doc();
    if d.root().is_none() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 4);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Nur HTML-Seiten haben einen Dokumentbaum."));
        ui.box_end();
        return;
    }
    // the first look: what holds the page is open
    if !doc.tree_seen {
        doc.tree_seen = true;
        for n in d.select("html, body") {
            doc.tree.expanded.insert(n.raw());
        }
    }
    let lines = tree_rows(d, &doc.tree, LINES_MAX);
    let rows: Vec<TreeRow> = lines.iter().map(|l| TreeRow { key: l.key, depth: l.depth, has_children: l.children, check: None, icon: 0, label: &l.label, detail: None }).collect();
    let chosen = doc.tree.cur.map(MdNode::from_raw).filter(|n| d.alive(*n) && !d.is_text(*n));
    let rect = ui.box_rect();
    let wide = ui.px2dip(rect[2]) >= 760;
    if chosen.is_some() {
        let total = ui.px2dip(if wide { rect[2] } else { rect[3] });
        if dev.inner == 0 && total > 0 {
            dev.inner = total * if wide { 58 } else { 45 } / 100;
        }
        ui.next_size(-1, -1);
        ui.split_begin(base + 0x10, dev.inner.max(80), !wide);
    }
    ui.next_size(-1, -1);
    let ev = ui.tree(base + 0x30, &mut doc.tree, &rows, 0);
    if ev.sel_changed || ev.expanded.is_some() || ev.collapsed.is_some() {
        ui.repass();
    }
    if let Some(n) = chosen {
        ui.split_next();
        let d = doc.view.doc();
        let (attrs, styles) = element_lines(d, n);
        ui.next_size(-1, -1);
        ui.scroll_begin(base + 0x18, SC_V);
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | pad(4), 3);
        caption(ui, &format!("<{}>", d.tag(n)));
        caption(ui, t("Attribute"));
        if attrs.is_empty() {
            ui.text(TS_CAPTION | TX_SECONDARY, t("Keine."));
        }
        for a in &attrs {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_WRAP, a);
        }
        caption(ui, t("Stile aus dem Stylesheet"));
        if styles.is_empty() {
            ui.text(TS_CAPTION | TX_SECONDARY, t("Keine."));
        }
        for a in &styles {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_WRAP, a);
        }
        ui.box_end();
        ui.scroll_end();
        ui.split_end(&mut dev.inner);
    }
}

// the table of the page's cookies
fn cookies_tab(ui: &mut Ui, base: u32, s: &mut Browser, url: &str, clock: i64) {
    let cookies = page_cookies(&s.jar, url, clock);
    let keys: Vec<u64> = cookies.iter().map(|c| c.order).collect();
    s.dev.cookies.retain(&keys);
    let mut act = 0;
    if cookies.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 4);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Diese Seite hat keine Cookies."));
        ui.box_end();
        return;
    }
    let cols = [
        GridCol::new(t("Name"), 90).flags(GC_NOSORT),
        GridCol::new(t("Wert"), 120).flags(GC_NOSORT | GC_FILL),
        GridCol::new(t("Domain"), 100).flags(GC_NOSORT),
        GridCol::new(t("Pfad"), 56).flags(GC_NOSORT),
        GridCol::new(t("Läuft ab"), 168).flags(GC_NOSORT),
        GridCol::new(t("Merkmale"), 120).flags(GC_NOSORT),
    ];
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    ui.next_size(-1, -1);
    let (first, end) = ui.grid_begin(base + 0x2C, &mut s.dev.cookies, &cols, &keys);
    for i in first..end {
        let c = cookies[i as usize];
        ui.grid_row(&mut s.dev.cookies, i);
        ui.grid_text(&s.dev.cookies, 0, &c.name);
        ui.grid_text(&s.dev.cookies, 1, &c.value);
        ui.grid_text(&s.dev.cookies, 2, &if c.host_only { c.domain.clone() } else { format!(".{}", c.domain) });
        ui.grid_text(&s.dev.cookies, 3, &c.path);
        ui.grid_text(&s.dev.cookies, 4, &expiry_text(c));
        ui.grid_text(&s.dev.cookies, 5, &flags_text(c));
    }
    ui.grid_end(&mut s.dev.cookies);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.spacer(-1);
    let picked = s.dev.cookies.cur.is_some();
    ui.next_flags(ST_COMPACT | ST_SUBTLE | if picked { 0 } else { ST_DISABLED });
    if ui.button(base + 0x20, t("Cookie löschen")) & EV_ACT != 0 {
        act = 1;
    }
    ui.next_flags(ST_COMPACT | ST_SUBTLE);
    if ui.button(base + 0x21, t("Alle löschen")) & EV_ACT != 0 {
        act = 2;
    }
    ui.box_end();
    ui.box_end();
    let gone: Vec<(String, String, String)> = match act {
        1 => cookies.iter().filter(|c| s.dev.cookies.cur == Some(c.order)).map(|c| (c.name.clone(), c.domain.clone(), c.path.clone())).collect(),
        2 => cookies.iter().map(|c| (c.name.clone(), c.domain.clone(), c.path.clone())).collect(),
        _ => Vec::new(),
    };
    if !gone.is_empty() {
        let j = alloc::sync::Arc::make_mut(&mut s.jar);
        for (n, d, p) in gone {
            j.remove(&n, &d, &p);
        }
        ui.repass();
    }
}

/// the pane below the page (see the module header) -> the pane wants to close
pub(crate) fn pane(ui: &mut Ui, base: u32, con_id: u32, s: &mut Browser, doc: &alloc::rc::Rc<core::cell::RefCell<Doc>>, clock: i64) -> bool {
    let mut close = false;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(4), 4);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    let names = [t("Konsole"), t("Netzwerk"), t("Elemente"), t("Cookies")];
    ui.next_size(-1, 0);
    if ui.tabs(base + 6, &mut s.dev.tab, &names) & EV_ACT != 0 {
        s.dev.inner = 0;
        ui.repass();
    }
    ui.next_icon(IC_CHROME_CLOSE);
    ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
    if ui.button(base + 4, t("Entwickleransicht schließen")) & EV_ACT != 0 {
        close = true;
    }
    ui.box_end();
    match s.dev.tab {
        TAB_NET => {
            let d = doc.borrow();
            net_tab(ui, base, &mut s.dev, &d.net);
        }
        TAB_ELEMENTS => {
            let mut d = doc.borrow_mut();
            elements_tab(ui, base, &mut s.dev, &mut d);
        }
        TAB_COOKIES => {
            let url = doc.borrow().url.clone();
            cookies_tab(ui, base, s, &url, clock);
        }
        _ => {
            let mut d = doc.borrow_mut();
            let clear = super::pagescript::console_pane(ui, con_id + 1, d.console(), &mut s.con_seen);
            if clear {
                d.clear_console();
                ui.repass();
            }
        }
    }
    ui.box_end();
    close
}
