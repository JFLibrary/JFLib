// webdevtest -- checks of the Webbrowser's B4 (group webdev of --selftest): style sheets and how the
// browser takes their answers, cookies over the network with every attribute and on a virtual clock,
// HttpOnly against document.cookie, Secure, SameSite, Set-Cookie on redirects and the cookie of the next
// hop, simulated HTTPS (each fault of a certificate, going on with one, curl -k, the handshake on the
// wire and in the inspector's words, a server without TLS, a script with TLS), the network log, the
// document tree of the developer view, two runs alike and a replay over the recording
//
// The network is fixtures/web_dev.jfnet: PC1 (10.0.0.10, name server DNS1), SW1, WEB1 (10.0.0.1: www on
// port 80, the script konto on 8080, sicher on 443 with a certificate of the lab CA, selbst 8441, alt
// 8442, anders 8443, spaeter 8444 with a fault each, the script echo on 8445, umleiter on 8081), DNS1.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use jfn_model::project::Project;
use jfn_sim::keyframe::digest;
use jfn_sim::{Action, Budget, Cmd, Engine, Inline, Note, Report, WebErr, WebFail, WebReq, WebResp, SEC};
use netpkt::cookie::Jar;
use netpkt::decode;
use netpkt::x509::{F_EXPIRED, F_NAME, F_NOT_YET, F_SELF_SIGNED};
use ui::{MdEv, MdNode, MdViewState};

use crate::selftest::T;
use crate::workspace::devwin::browser::{fault_lines, sheet_text, store_cookies};
use crate::workspace::devwin::devtools::{body_text, element_lines, net_answer, page_cookies, pending_row, rows_of, tree_rows, Who};
use crate::workspace::devwin::pagescript::PageScript;

const SITE: &str = include_str!("../tests/fixtures/web_dev.jfnet");
const BUDGET: usize = 8_000_000;
const PC1: u32 = 1;
const WEB1: u32 = 3;

fn s(x: &str) -> String {
    String::from(x)
}

fn project() -> Project {
    Project::from_text(SITE).unwrap_or_else(|_| Project::new("web", 1))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

fn engine() -> Engine {
    let mut e = Engine::from_project(&project()).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut e, 2 * SEC);
    e
}

// the browser without a window: its jar, the certificates it went on with
#[derive(Default)]
struct Client {
    jar: Arc<Jar>,
    accept: Vec<[u8; 32]>,
    next: u64,
    /// who asks: the site of the page ("" the user), a navigation
    site: String,
    top: bool,
    refused: Vec<String>,
}

impl Client {
    fn fetch(&mut self, e: &mut Engine, method: &str, url: &str) -> Option<Result<WebResp, WebFail>> {
        self.next += 1;
        let id = self.next;
        let req = WebReq { id, method: s(method), url: s(url), jar: Some(self.jar.clone()), site: self.site.clone(), top: self.top, accept: self.accept.clone(), ..WebReq::default() };
        let _ = e.take_reports();
        if e.apply(Action::WebFetch { dev: PC1, req }).is_err() {
            return None;
        }
        run_for(e, SEC);
        let mut out = None;
        for r in e.take_reports() {
            if let Report::Web { id: i, result, .. } = r {
                if i == id {
                    let hops = match &result {
                        Ok(a) => a.hops.clone(),
                        Err(f) => f.hops.clone(),
                    };
                    self.refused = store_cookies(&mut self.jar, &hops);
                    out = Some(result);
                }
            }
        }
        out
    }

    fn get(&mut self, e: &mut Engine, url: &str) -> Option<Result<WebResp, WebFail>> {
        self.fetch(e, "GET", url)
    }

    fn body(&mut self, e: &mut Engine, url: &str) -> String {
        match self.get(e, url) {
            Some(Ok(a)) => String::from_utf8_lossy(&a.body).into_owned(),
            Some(Err(f)) => format!("{:?}", f.kind),
            None => s("nichts"),
        }
    }
}

fn clock(e: &Engine) -> i64 {
    e.clock(PC1).unwrap_or(0).div_euclid(1_000_000_000)
}

fn has(jar: &Jar, name: &str) -> bool {
    jar.cookies().iter().any(|c| c.name == name)
}

// ---------------------------------------------------------------- style sheets

fn sheets(t: &mut T) {
    let mut e = engine();
    let mut c = Client::default();
    // the fixture's page names them in the order they count
    let page = c.body(&mut e, "http://web.lab/");
    let mut view = MdViewState::new();
    view.set_html(&page);
    t.eq("the links of the page", view.doc().sheet_links().to_vec(), alloc::vec![s("basis.css"), s("ende.css")]);
    let col = |v: &MdViewState, sel: &str| {
        let d = v.doc();
        let n = d.select_first(sel).unwrap_or_default();
        d.blocks().iter().find(|b| b.node == n).and_then(|b| b.spans.first()).map_or(0, |x| x.color & 0xFF_FFFF)
    };
    t.eq("without its sheets: the <style> alone", col(&view, "p.mitte"), 0x1C5A8A);
    // the answers as the browser takes them
    let basis = c.get(&mut e, "http://web.lab/basis.css");
    let text = basis.as_ref().and_then(|r| sheet_text(r, "http://web.lab/basis.css").ok());
    t.check("a sheet is text/css", text.is_some());
    view.doc_mut().set_sheet("basis.css", text.as_deref());
    view.update();
    t.eq("the first link before the <style>: the <style> wins", col(&view, "p.mitte"), 0x1C5A8A);
    t.eq("h1 too", col(&view, "h1"), 0x8A1C1C);
    t.eq("what only the sheet says", col(&view, "p.ende"), 0x333333);
    let ende = c.get(&mut e, "http://web.lab/ende.css");
    let text = ende.as_ref().and_then(|r| sheet_text(r, "http://web.lab/ende.css").ok());
    view.doc_mut().set_sheet("ende.css", text.as_deref());
    view.update();
    t.eq("the last link after the <style>: it wins", col(&view, "p.mitte"), 0x2E7D32);
    t.check("a box from a sheet", !view.doc().boxes().is_empty());
    let n = view.doc().select_first("p.mitte").unwrap_or_default();
    t.check("style_of says what it got", view.doc().style_of(n).contains("color:#2E7D32"));
    // what fails is a line for the console
    let line = |r: Option<Result<WebResp, WebFail>>, url: &str| r.and_then(|x| sheet_text(&x, url).err()).unwrap_or_default();
    let a = c.get(&mut e, "http://web.lab/fehlt.css");
    t.check("a sheet that is missing", line(a, "http://web.lab/fehlt.css").contains("404"));
    let b = c.get(&mut e, "http://web.lab/notiz.txt");
    t.check("a text that is no sheet", line(b, "http://web.lab/notiz.txt").contains("text/plain"));
    let d = c.get(&mut e, "http://web.lab:81/zu.css");
    t.check("a server that is not there", !line(d, "http://web.lab:81/zu.css").is_empty());
    let big = WebResp { status: 200, body: alloc::vec![b' '; (1 << 20) + 1], ..WebResp::default() };
    t.check("a sheet too large", sheet_text(&Ok(big), "http://web.lab/gross.css").err().is_some_and(|l| l.contains("zu groß")));
    let untyped = WebResp { status: 200, body: Vec::from(&b"p { color: red }"[..]), ..WebResp::default() };
    t.eq("no type at all: taken", sheet_text(&Ok(untyped), "http://web.lab/x").ok(), Some(s("p { color: red }")));
    // a page without sheets asks for nothing
    let page = c.body(&mut e, "http://web.lab/ohne.html");
    let mut v = MdViewState::new();
    v.set_html(&page);
    t.eq("no links, nothing to ask", v.doc().sheet_links().len(), 0);
}

// ---------------------------------------------------------------- cookies

fn cookies(t: &mut T) {
    let mut e = engine();
    let mut c = Client::default();
    t.eq("no cookies: no header", c.body(&mut e, "http://web.lab:8080/lesen"), s("Gesendet: \n"));
    let start = clock(&e);
    let _ = c.get(&mut e, "http://web.lab:8080/kekse");
    t.check("what fits goes into the jar", has(&c.jar, "sitzung") && has(&c.jar, "dauer") && has(&c.jar, "datum") && has(&c.jar, "nurhttp"));
    t.check("a Domain of another site does not", !has(&c.jar, "falsch"));
    t.check("Secure over http does not", !has(&c.jar, "sicher"));
    t.eq("both are refused with a reason", c.refused.len(), 2);
    t.check("the reason names the cookie", c.refused.iter().any(|l| l.contains("falsch")) && c.refused.iter().any(|l| l.contains("sicher")));
    let dauer = c.jar.cookies().iter().find(|x| x.name == "dauer").cloned().unwrap_or_default();
    t.check("Max-Age counts from the device clock", dauer.expires.is_some_and(|x| x > start && x <= start + 62));
    let datum = c.jar.cookies().iter().find(|x| x.name == "datum").cloned().unwrap_or_default();
    t.eq("Expires as a date", datum.expires, Some(3_155_760_000));
    t.check("HttpOnly as an attribute", c.jar.cookies().iter().any(|x| x.name == "nurhttp" && x.http_only));
    t.check("a session cookie has no date", c.jar.cookies().iter().any(|x| x.name == "sitzung" && x.expires.is_none()));
    t.eq("all go with a request", c.body(&mut e, "http://web.lab:8080/lesen"), s("Gesendet: sitzung=abc; dauer=lang; datum=fest; nurhttp=geheim\n"));
    // another host does not get them
    t.eq("host-only: another name of the host", c.body(&mut e, "http://www.web.lab:8080/lesen"), s("Gesendet: \n"));
    // the path
    let _ = c.get(&mut e, "http://web.lab:8080/nurpfad");
    t.check("Path: elsewhere not", !c.body(&mut e, "http://web.lab:8080/lesen").contains("pfad=1"));
    t.check("Path: below it", c.body(&mut e, "http://web.lab:8080/pfad/lesen").contains("pfad=1"));
    // expiry on virtual time (the requests above took 5 s of the 60)
    run_for(&mut e, 50 * SEC);
    t.check("before its time", c.body(&mut e, "http://web.lab:8080/lesen").contains("dauer=lang"));
    run_for(&mut e, 5 * SEC);
    let x = c.body(&mut e, "http://web.lab:8080/lesen");
    t.check("at its time on the virtual clock", !x.contains("dauer=lang") && x.contains("sitzung=abc") && x.contains("datum=fest"));
    // the window's jar lives on: a session cookie goes when the browser ends, the one with a date stays
    let mut j = c.jar.clone();
    Arc::make_mut(&mut j).drop_session();
    t.check("the session ends", !has(&j, "sitzung") && has(&j, "datum"));
    // the page's cookies as the developer view lists them
    let now = clock(&e);
    let here = page_cookies(&c.jar, "http://web.lab:8080/x", now);
    t.check("the page's cookies, HttpOnly too", here.iter().any(|x| x.name == "nurhttp"));
    t.check("not those of another path", !here.iter().any(|x| x.name == "pfad"));
    t.eq("another host's page has none", page_cookies(&c.jar, "http://andere.lab/", now).len(), 0);
}

// a page with a script, a jar and a clock
fn page_of(html: &str, url: &str) -> Option<(MdViewState, Box<PageScript>)> {
    let mut view = MdViewState::new();
    view.set_html(html);
    let p = PageScript::of(&view, html, url, 1)?;
    Some((view, p))
}

fn script_cookies(t: &mut T) {
    let mut jar = Arc::new(Jar::new());
    let o = netpkt::cookie::Origin { host: "web.lab", path: "/", secure: false, http: true };
    let _ = Arc::make_mut(&mut jar).store("offen=1", o, 1000);
    let _ = Arc::make_mut(&mut jar).store("nurhttp=2; HttpOnly", o, 1000);
    let html = "<p id=\"a\">x</p><script type=\"text/jfs\">
print(document.cookie)
document.cookie = \"neu=3\"
print(\"nach der Zuweisung: \" + document.cookie)
document.set_cookie(\"gleich=4\")
print(\"nach set_cookie: \" + document.cookie)
document.set_cookie(\"hart=5; HttpOnly\")
document.set_cookie(\"nurhttp=6\")
document.set_cookie(\"tresor=7; Secure\")
timer.after(1, fn() { print(\"später: \" + document.cookie) })
</script>";
    let Some((mut view, mut p)) = page_of(html, "http://web.lab/seite.html") else { return t.check("the page", false) };
    let mut seq = 0;
    p.jar_in(&mut jar, 1000);
    p.pump(&mut view, SEC, &mut seq);
    p.jar_out(&mut jar);
    let out = p.console().text();
    let lines: Vec<&str> = out.lines().collect();
    t.eq("document.cookie: no HttpOnly", lines.first().copied(), Some("offen=1"));
    t.eq("an assignment shows at the end of the call", lines.get(1).copied(), Some("nach der Zuweisung: neu=3"));
    t.check("set_cookie at once", lines.get(2).is_some_and(|l| l.contains("gleich=4") && l.contains("offen=1")));
    t.check("the assignment reached the jar", has(&jar, "neu") && has(&jar, "gleich"));
    t.check("a script sets no HttpOnly", !has(&jar, "hart"));
    t.check("a script overwrites no HttpOnly", jar.cookies().iter().any(|c| c.name == "nurhttp" && c.value == "2"));
    t.check("a page without https sets no Secure", !has(&jar, "tresor"));
    t.eq("what it refused is in the console", out.matches("nicht gespeichert").count(), 3);
    p.jar_in(&mut jar, 1001);
    p.pump(&mut view, 2 * SEC, &mut seq);
    p.jar_out(&mut jar);
    let later = p.console().text();
    t.check("the next call reads all of it, no HttpOnly", later.lines().last().is_some_and(|l| l.contains("neu=3") && l.contains("gleich=4") && !l.contains("nurhttp")));
    // a handler reads the jar as it is now: a request's Set-Cookie between two clicks
    let _ = Arc::make_mut(&mut jar).store("danach=8", o, 1001);
    let html = "<p id=\"a\">x</p><script type=\"text/jfs\">
document.by_id(\"a\").on(\"click\", fn(ev) { print(document.cookie) })
</script>";
    let Some((mut view, mut p)) = page_of(html, "http://web.lab/") else { return t.check("the second page", false) };
    p.jar_in(&mut jar, 1001);
    p.pump(&mut view, SEC, &mut seq);
    p.jar_out(&mut jar);
    let n = view.doc().by_id("a").unwrap_or_default();
    p.jar_in(&mut jar, 1001);
    let ev = MdEv { click: Some(n), ..MdEv::default() };
    let _ = p.events(&mut view, ev, SEC, &mut seq);
    p.jar_out(&mut jar);
    t.check("an event's handler", p.console().text().contains("danach=8"));
    // a page over https sets Secure
    let html = "<script type=\"text/jfs\">document.set_cookie(\"tresor=7; Secure\")\nprint(document.cookie)</script>";
    let Some((mut view, mut p)) = page_of(html, "https://web.lab/") else { return t.check("the third page", false) };
    p.jar_in(&mut jar, 1001);
    p.pump(&mut view, SEC, &mut seq);
    p.jar_out(&mut jar);
    t.check("https: Secure goes", has(&jar, "tresor") && p.console().text().contains("tresor=7"));
    t.check("http: it is not read", !jar.script_text("web.lab", "/", false, 1001).contains("tresor"));
}

fn redirects(t: &mut T) {
    let mut e = engine();
    let mut c = Client::default();
    // Set-Cookie on the redirect, the cookie of the next hop
    let r = c.get(&mut e, "http://web.lab:8080/login");
    let a = r.and_then(|x| x.ok());
    t.eq("the login's answer", a.as_ref().map(|x| x.status), Some(200));
    t.check("the next hop has the cookie", a.as_ref().is_some_and(|x| String::from_utf8_lossy(&x.body).contains("anmeldung=frisch")));
    let hops = a.as_ref().map(|x| x.hops.clone()).unwrap_or_default();
    t.eq("two hops", hops.iter().map(|h| h.status).collect::<Vec<u16>>(), alloc::vec![302, 200]);
    t.eq("the first went without", hops.first().and_then(|h| h.sent.get("cookie").map(String::from)), None);
    t.eq("the second with it", hops.get(1).and_then(|h| h.sent.get("cookie").map(String::from)), Some(s("anmeldung=frisch")));
    t.check("the window's jar has it", has(&c.jar, "anmeldung"));
    // a chain that sets and deletes on the way
    let mut c = Client::default();
    let r = c.get(&mut e, "http://web.lab:8080/kette");
    let a = r.and_then(|x| x.ok());
    t.eq("set, deleted, what the last hop got", a.as_ref().map(|x| String::from_utf8_lossy(&x.body).into_owned()), Some(s("Gesendet: zwei=2\n")));
    t.eq("three hops", a.as_ref().map(|x| x.hops.len()), Some(3));
    t.check("the jar ends the same", !has(&c.jar, "eins") && has(&c.jar, "zwei"));
    t.eq("the request's own copy did not touch the snapshot", Client::default().jar.cookies().len(), 0);
    // a request without a jar sends and keeps none (curl's way)
    let _ = e.apply(Action::WebFetch { dev: PC1, req: WebReq { id: 900, method: s("GET"), url: s("http://web.lab:8080/login"), ..WebReq::default() } });
    run_for(&mut e, SEC);
    let body = e.take_reports().into_iter().find_map(|r| match r {
        Report::Web { id: 900, result: Ok(a), .. } => Some(a),
        _ => None,
    });
    t.eq("no jar: the next hop has none either", body.map(|a| a.status), Some(401));
    // SameSite across sites
    let mut c = Client::default();
    let _ = c.body(&mut e, "https://web.lab:8445/setzen");
    let at = |c: &mut Client, e: &mut Engine, site: &str, top: bool| {
        c.site = s(site);
        c.top = top;
        c.body(e, "https://web.lab:8445/lesen")
    };
    let all = at(&mut c, &mut e, "", true);
    t.check("the user: all", all.contains("tresor=1") && all.contains("strikt=2") && all.contains("breit=3"));
    let inside = at(&mut c, &mut e, "web.lab", false);
    t.check("the same site: all", inside.contains("tresor=1") && inside.contains("strikt=2") && inside.contains("breit=3"));
    let cross = at(&mut c, &mut e, "other.lab", false);
    t.eq("another site, a picture: only None", cross, s("Über TLS: breit=3\n"));
    let link = at(&mut c, &mut e, "other.lab", true);
    t.check("another site, a link: no Strict", link.contains("tresor=1") && !link.contains("strikt") && link.contains("breit=3"));
}

// ---------------------------------------------------------------- HTTPS

fn tls_of(r: &Option<Result<WebResp, WebFail>>) -> Option<jfn_sim::webc::TlsInfo> {
    match r {
        Some(Ok(a)) => a.tls.clone(),
        Some(Err(f)) => f.tls.clone(),
        None => None,
    }
}

fn https(t: &mut T) {
    let mut e = engine();
    let mut c = Client::default();
    // a good certificate
    let r = c.get(&mut e, "https://web.lab/");
    let i = tls_of(&r);
    let a = r.and_then(|x| x.ok());
    t.eq("https: an answer", a.as_ref().map(|x| x.status), Some(200));
    t.eq("the final URL says https", a.as_ref().map(|x| x.url.clone()), Some(s("https://web.lab/")));
    t.check("the page", a.as_ref().is_some_and(|x| String::from_utf8_lossy(&x.body).contains("Hofladen")));
    t.eq("no faults", i.as_ref().map(|x| x.faults), Some(0));
    let cert = i.as_ref().and_then(|x| x.chain.first().cloned()).unwrap_or_default();
    t.eq("its subject", cert.subject_cn.as_str(), "web.lab");
    t.eq("its organization", cert.subject_org.as_str(), "Hofladen Sonnenfeld");
    t.eq("its issuer", cert.issuer_cn.as_str(), "JFNetworker Lab CA");
    t.eq("its names", cert.name_list(), alloc::vec![s("web.lab"), s("www.web.lab")]);
    t.eq("the version", i.as_ref().map(|x| x.version.clone()), Some(s("TLS 1.3")));
    t.eq("the suite", i.as_ref().map(|x| x.suite.clone()), Some(s("TLS_CHACHA20_POLY1305_SHA256")));
    let host = a.as_ref().and_then(|x| x.hops.first()).and_then(|h| h.sent.get("host").map(String::from));
    t.eq("Host without the default port", host, Some(s("web.lab")));
    // another name of the certificate, a port of its own in the URL
    t.eq("another of its names", c.get(&mut e, "https://www.web.lab/").and_then(|x| x.ok()).map(|x| x.status), Some(200));
    // each fault
    for (name, url, fault) in [("self-signed", "https://web.lab:8441/", F_SELF_SIGNED), ("expired", "https://web.lab:8442/", F_EXPIRED), ("another name", "https://web.lab:8443/", F_NAME), ("not yet", "https://web.lab:8444/", F_NOT_YET)] {
        let mut k = Client::default();
        let r = k.get(&mut e, url);
        let f = r.as_ref().and_then(|x| x.as_ref().err()).cloned();
        t.eq(&format!("{}: refused", name), f.as_ref().map(|x| x.kind), Some(WebErr::Cert));
        t.eq(&format!("{}: its fault", name), f.as_ref().and_then(|x| x.tls.as_ref()).map(|x| x.faults), Some(fault));
        let info = f.as_ref().and_then(|x| x.tls.clone()).unwrap_or_default();
        t.eq(&format!("{}: a sentence for the page", name), fault_lines(&info, "web.lab").len(), 1);
        // the user goes on: that one certificate
        let fp = info.chain.first().map(|x| x.fingerprint()).unwrap_or([0; 32]);
        k.accept.push(fp);
        let again = k.get(&mut e, url);
        let i = tls_of(&again);
        t.eq(&format!("{}: goes on when accepted", name), again.and_then(|x| x.ok()).map(|x| x.status), Some(200));
        t.check(&format!("{}: excused", name), i.as_ref().is_some_and(|x| x.excused && x.faults == fault));
        // another certificate is not that one
        let mut other = Client { accept: alloc::vec![[7; 32]], ..Client::default() };
        t.eq(&format!("{}: another fingerprint does not", name), other.get(&mut e, url).and_then(|x| x.err()).map(|x| x.kind), Some(WebErr::Cert));
    }
    t.check("the refusal is a note", e.trace.events().any(|x| x.dev == PC1 && matches!(&x.note, Note::Tls(jfn_sim::tlssim::TlsNote::CertRefused { .. }))));
    t.check("the server hears the alert", e.trace.events().any(|x| x.dev == WEB1 && matches!(&x.note, Note::Tls(jfn_sim::tlssim::TlsNote::Failed { ours: false, .. }))));
    // no TLS on that port, TLS on a port for http
    let r = c.get(&mut e, "https://web.lab:80/");
    t.eq("a port without TLS", r.and_then(|x| x.err()).map(|x| x.kind), Some(WebErr::Tls));
    t.eq("http to a port with TLS", c.get(&mut e, "http://web.lab:443/").and_then(|x| x.err()).map(|x| x.kind), Some(WebErr::Broken));
    // redirects across the schemes, Secure on the way
    let r = c.get(&mut e, "http://web.lab:8081/");
    t.eq("http to https", r.and_then(|x| x.ok()).map(|x| (x.url, x.redirects)), Some((s("https://web.lab/"), 1)));
    let r = c.get(&mut e, "https://web.lab/hin");
    t.eq("https to http", r.and_then(|x| x.ok()).map(|x| x.url), Some(s("http://web.lab:8080/lesen")));
    let mut k = Client::default();
    let _ = k.get(&mut e, "https://web.lab/tresor");
    t.check("Secure over https", has(&k.jar, "tresor"));
    t.eq("it goes over https", k.body(&mut e, "https://web.lab:8445/"), s("Über TLS: tresor=auf\n"));
    t.eq("not over http", k.body(&mut e, "http://web.lab:8080/lesen"), s("Gesendet: \n"));
    // a script with TLS: it answered above; the service says TLS
    t.check("the service says TLS", e.service_status(WEB1, "sicher").is_some_and(|x| x.contains("(TLS)")));
    t.check("its metrics", e.metric(WEB1, "sicher", "tls_handshakes").is_some_and(|n| n >= 2));
    t.check("failures count", e.metric(WEB1, "selbst", "tls_failed").is_some_and(|n| n >= 2));
    // a restart keeps the certificate
    let fp1 = tls_of(&c.get(&mut e, "https://web.lab/")).and_then(|i| i.chain.first().map(|c| c.fingerprint()));
    let mut f = engine();
    let fp2 = tls_of(&Client::default().get(&mut f, "https://web.lab/")).and_then(|i| i.chain.first().map(|c| c.fingerprint()));
    t.check("the same project, the same certificate", fp1.is_some() && fp1 == fp2);
}

// the handshake on the wire, as the inspector reads it
fn wire(t: &mut T) {
    let mut e = engine();
    let mut c = Client::default();
    let _ = c.get(&mut e, "https://web.lab/");
    let lines: Vec<(u32, String)> = e.trace.hops().filter(|h| h.mask & decode::P_TLS != 0).map(|h| (h.from.0, decode::info(&h.bytes).1)).collect();
    t.check("TLS on the wire", !lines.is_empty());
    t.check("the client's hello", lines.iter().any(|l| l.0 == PC1 && l.1.starts_with("Client Hello (web.lab)")));
    t.check("the server's hello and what is hidden", lines.iter().any(|l| l.0 == WEB1 && l.1.starts_with("Server Hello, verschlüsselt (")));
    t.check("what follows is hidden", lines.iter().any(|l| l.0 == PC1 && l.1.starts_with("verschlüsselt (")));
    let layered = e.trace.hops().filter(|h| h.mask & decode::P_TLS != 0).find_map(|h| {
        let ls = decode::layers(&h.bytes);
        ls.into_iter().find(|l| l.proto == decode::Proto::Tls && l.fields.iter().any(|f| f.name == "Cipher Suites"))
    });
    t.check("the hello's fields", layered.is_some_and(|l| l.fields.iter().any(|f| f.name == "Extension server_name" && f.value == "web.lab")));
    // nothing of the page or the request shows
    let clear = e.trace.hops().any(|h| h.mask & decode::P_TLS != 0 && h.bytes.windows(5).any(|w| w == b"GET /" || w == b"Hofla"));
    t.check("nothing in the clear", !clear);
    t.check("no HTTP layer there", !e.trace.hops().any(|h| h.mask & decode::P_TLS != 0 && h.mask & decode::P_HTTP != 0));
    t.check("the notes of the handshake", e.trace.events().any(|x| x.dev == WEB1 && matches!(&x.note, Note::Tls(jfn_sim::tlssim::TlsNote::Hello { .. }))) && e.trace.events().any(|x| x.dev == PC1 && matches!(&x.note, Note::Tls(jfn_sim::tlssim::TlsNote::Established { .. }))));
    // a kept-alive connection: the second request has no handshake of its own
    let before = e.trace.hops().filter(|h| decode::info(&h.bytes).1.starts_with("Client Hello")).count();
    let _ = c.get(&mut e, "https://web.lab/basis.css");
    let after = e.trace.hops().filter(|h| decode::info(&h.bytes).1.starts_with("Client Hello")).count();
    t.eq("a kept-alive connection", after, before);
}

// ---------------------------------------------------------------- the developer view's parts

fn log(t: &mut T) {
    let mut e = engine();
    let mut c = Client::default();
    let _ = c.get(&mut e, "http://web.lab:8080/kekse");
    let r = c.get(&mut e, "http://web.lab:8080/login");
    let result = r.unwrap_or(Err(WebFail { kind: WebErr::Stopped, url: String::new(), msg: jfn_model::msg!("-"), hops: Vec::new(), tls: None }));
    let rows = rows_of(7, Who::Link, 1000, &result);
    t.eq("a row for every hop", rows.iter().map(|x| x.status).collect::<Vec<u16>>(), alloc::vec![302, 200]);
    t.eq("their keys", rows.iter().map(|x| x.key).collect::<Vec<u64>>(), alloc::vec![7 << 8, 7 << 8 | 1]);
    t.eq("the addresses", rows.first().map(|x| x.url.clone()), Some(s("http://web.lab:8080/login")));
    t.eq("the method", rows.first().map(|x| x.method.clone()), Some(s("GET")));
    t.eq("who asked", rows.first().map(|x| x.who), Some(Who::Link));
    t.check("the request's head with the cookie", rows.first().is_some_and(|x| x.sent.get("cookie").is_some_and(|v| v.contains("sitzung=abc"))));
    t.eq("the Host first", rows.first().and_then(|x| x.sent.0.first().map(|h| h.0.clone())), Some(s("Host")));
    t.check("the answer's head", rows.first().is_some_and(|x| x.headers.get("location") == Some("/konto")));
    t.eq("the last one has the type", rows.get(1).map(|x| x.ctype.clone()), Some(s("text/html")));
    t.check("the last one has the body", rows.get(1).is_some_and(|x| x.size > 0 && body_text(x).contains("Willkommen")));
    t.check("the time goes on", rows.get(1).is_some_and(|x| x.start > 1000));
    // a failure
    let f = c.get(&mut e, "http://web.lab:81/").unwrap_or(Err(WebFail { kind: WebErr::Stopped, url: String::new(), msg: jfn_model::msg!("-"), hops: Vec::new(), tls: None }));
    let rows = rows_of(8, Who::Sheet, 0, &f);
    t.eq("a failure is one row", rows.len(), 1);
    t.check("it says why", rows.first().is_some_and(|x| x.status == 0 && !x.failed.is_empty()));
    // pending, then answered in its place
    let req = WebReq { id: 3, method: s("GET"), url: s("http://web.lab/a"), ..WebReq::default() };
    let mut net = alloc::vec![pending_row(&WebReq { id: 2, ..req.clone() }, Who::Image, 10), pending_row(&req, Who::Fetch, 11), pending_row(&WebReq { id: 4, ..req.clone() }, Who::Image, 12)];
    t.check("on its way", net[1].pending && net[1].who == Who::Fetch);
    net_answer(&mut net, 3, rows_of(3, Who::Fetch, 11, &result));
    t.eq("the answers in its place", net.iter().map(|x| x.req).collect::<Vec<u64>>(), alloc::vec![2, 3, 3, 4]);
    // a picture shows as bytes
    let img = crate::workspace::devwin::devtools::NetRow { ctype: s("image/png"), size: 4, body: alloc::vec![0x89, 0x50, 0x4E, 0x47], ..Default::default() };
    t.check("binary: the first bytes", body_text(&img).contains("89 50 4e 47"));
    // the tree: what is open, live, the element's parts
    let mut v = MdViewState::new();
    v.set_html("<html><head><title>T</title><style>.r { color: #AA0000 } #e { font-weight: bold }</style></head><body><ul id=\"l\"><li id=\"e\" class=\"r\" data-x=\"1\">Eins</li></ul>\n<p>  </p></body></html>");
    let mut tree = ui::TreeState::default();
    let lines = tree_rows(v.doc(), &tree, 100);
    t.eq("closed: the top", lines.iter().map(|l| l.label.clone()).collect::<Vec<String>>(), alloc::vec![s("<html>")]);
    t.check("it has children", lines[0].children);
    for n in v.doc().select("html, body, ul") {
        tree.expanded.insert(n.raw());
    }
    let lines = tree_rows(v.doc(), &tree, 100);
    let labels: Vec<String> = lines.iter().map(|l| l.label.clone()).collect();
    t.check("with its attributes", labels.iter().any(|l| l.contains("<li id=\"e\" class=\"r\" data-x=\"1\">")));
    t.check("depth", lines.iter().find(|l| l.label.starts_with("<li")).is_some_and(|l| l.depth == 3));
    t.check("blank texts are left out", !labels.iter().any(|l| l == "\"\""));
    let n = v.doc().by_id("e").unwrap_or_default();
    let (attrs, styles) = element_lines(v.doc(), n);
    t.eq("its attributes", attrs, alloc::vec![s("id = \"e\""), s("class = \"r\""), s("data-x = \"1\"")]);
    t.eq("its styles", styles, alloc::vec![s("color: #AA0000"), s("font-weight: bold")]);
    // live: a change shows in the next lines
    let l = v.doc().by_id("l").unwrap_or_default();
    let d = v.doc_mut();
    let x = d.create_element("li");
    d.set_attr(x, "data-neu", "ja");
    d.insert(l, None, x);
    let labels: Vec<String> = tree_rows(v.doc(), &tree, 100).iter().map(|l| l.label.clone()).collect();
    t.check("live: an element a script adds", labels.iter().any(|l| l.contains("data-neu")));
    t.eq("at most so many lines", tree_rows(v.doc(), &tree, 2).len(), 2);
    t.eq("no such node", element_lines(v.doc(), MdNode::NONE), (Vec::new(), Vec::new()));
    // Markdown has no tree
    let mut m = MdViewState::new();
    m.set_markdown("# x");
    t.eq("Markdown: no lines", tree_rows(m.doc(), &tree, 10).len(), 0);
}

// ---------------------------------------------------------------- the same again

fn session(e: &mut Engine) -> (String, Vec<String>) {
    let mut c = Client::default();
    let mut out = Vec::new();
    for u in ["http://web.lab:8080/kekse", "http://web.lab:8080/login", "https://web.lab/", "https://web.lab:8441/", "https://web.lab:8445/setzen", "http://web.lab/basis.css"] {
        let r = c.get(e, u);
        out.push(format!("{:?}", r.map(|x| x.map(|a| (a.status, a.url, a.hops.len(), a.tls.map(|i| i.chain.first().map(|c| c.fingerprint())))).map_err(|f| (f.kind, f.url)))));
    }
    (digest(e).to_string(), out)
}

fn determinism(t: &mut T) {
    let (mut a, mut b) = (engine(), engine());
    let x = session(&mut a);
    let y = session(&mut b);
    t.eq("two runs: the same state", x.0, y.0);
    t.eq("two runs: the same answers", x.1, y.1);
    // the bytes on the wire are the same too
    let wire = |e: &Engine| e.trace.hops().filter(|h| h.mask & decode::P_TLS != 0).map(|h| h.bytes.to_vec()).collect::<Vec<Vec<u8>>>();
    t.check("two runs: the same TLS bytes", !wire(&a).is_empty() && wire(&a) == wire(&b));
    // a recorded run replays: the jar goes with the action, a Set-Cookie on the way too
    let mut d = Inline::new(Engine::from_project(&project()).unwrap_or_else(|_| Engine::new(0)));
    d.rewind_on(Budget::default());
    let _ = d.send(Cmd::RunFor(2 * SEC), 0);
    let mut jar = Jar::new();
    let o = netpkt::cookie::Origin { host: "web.lab", path: "/", secure: false, http: true };
    let _ = jar.store("mit=1", o, 0);
    let jar = Arc::new(jar);
    for (id, url) in [(1, "http://web.lab:8080/login"), (2, "https://web.lab/"), (3, "https://web.lab:8441/")] {
        let req = WebReq { id, method: s("GET"), url: s(url), jar: Some(jar.clone()), ..WebReq::default() };
        let _ = d.send(Cmd::Apply(Action::WebFetch { dev: PC1, req }), 0);
        let _ = d.send(Cmd::RunFor(SEC), 0);
    }
    let live = digest(&d.eng);
    let reports = d.reports();
    t.eq("three reports", reports.iter().filter(|r| matches!(r, Report::Web { .. })).count(), 3);
    let answer = reports.iter().find_map(|r| match r {
        Report::Web { id: 1, result: Ok(a), .. } => Some(a.clone()),
        _ => None,
    });
    t.eq("the jar went with the action", answer.map(|a| String::from_utf8_lossy(&a.body).into_owned()), Some(s("<html><head><title>Willkommen</title></head><body><h1>Willkommen</h1><p>Angemeldet mit: mit=1; anmeldung=frisch</p></body></html>")));
    let pos = d.pos();
    let replayed = d.restore_pos(pos, None).and_then(|mut r| {
        let rw = d.rewind()?;
        r.run(rw, usize::MAX).then(|| r.finish())
    });
    t.eq("a replay ends in the same state", replayed.map(|p| digest(&p.eng)), Some(live));
}

pub(crate) fn run(t: &mut T) {
    sheets(t);
    cookies(t);
    script_cookies(t);
    redirects(t);
    https(t);
    wire(t);
    log(t);
    determinism(t);
}
