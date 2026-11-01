// browsertest -- checks of the Webbrowser (group browser of --selftest: netpkt::uri, jfn_sim::webc,
// the static files of jfn_sim::httpsvc, req.form of jfn_sim::scriptsvc, what devwin/browser.rs
// decides): URL parsing and resolution with every example of RFC 3986 5.4, dot segments, percent
// and form encoding, the address line, each error kind, the redirect rules and their limit,
// content types, forms with GET and POST against a script, static files, the history, fragments,
// a cancelled request, the power going off under a request, a replay and determinism
//
// The network is fixtures/web_site.jfnet: PC1 (10.0.0.10, no gateway, name server DNS1), SW1, WEB1
// (10.0.0.1: www on port 80 with "root": "/var/www", the script laden on port 8080), DNS1
// (10.0.0.53: the zone lab with web.lab).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::Project;
use jfn_sim::keyframe::digest;
use jfn_sim::webc::{kind_of, redirected, CLIENT_SVC, MAX_REDIRECTS};
use jfn_sim::{Action, Budget, Cmd, Engine, Inline, Report, WebErr, WebFail, WebReq, WebResp, MS, SEC};
use netpkt::http::{mime_of, Headers};
use netpkt::uri::{self, remove_dot_segments, Uri};
use ui::{MdDoc, MdSubmit};

use crate::selftest::T;
use crate::workspace::devwin::browser::{address, block_of, classify, data_url, error_page, esc, form_nav, Hist, Kind, Nav, HIST_MAX};

const SITE: &str = include_str!("../tests/fixtures/web_site.jfnet");
const BUDGET: usize = 8_000_000;
const PC1: u32 = 1;
const WEB1: u32 = 3;
const DNS1: u32 = 4;

fn s(x: &str) -> String {
    String::from(x)
}

fn project() -> Project {
    Project::from_text(SITE).unwrap_or_else(|_| Project::new("web", 1))
}

fn engine() -> Engine {
    let mut e = Engine::from_project(&project()).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut e, 2 * SEC);
    e
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

fn req(id: u64, method: &str, url: &str, ctype: &str, body: &str) -> WebReq {
    let mut headers = Headers::new();
    if !ctype.is_empty() {
        headers.add("Content-Type", ctype);
    }
    WebReq { id, method: s(method), url: s(url), headers, body: Vec::from(body.as_bytes()), ..WebReq::default() }
}

fn web_reports(e: &mut Engine) -> Vec<(u64, Result<WebResp, WebFail>)> {
    let mut out = Vec::new();
    for r in e.take_reports() {
        if let Report::Web { id, result, .. } = r {
            out.push((id, result));
        }
    }
    out
}

/// a request from `dev`, `d` of virtual time, its outcome
fn fetch_from(e: &mut Engine, dev: u32, r: WebReq, d: u64) -> Option<Result<WebResp, WebFail>> {
    let id = r.id;
    let _ = e.take_reports();
    if e.apply(Action::WebFetch { dev, req: r }).is_err() {
        return None;
    }
    run_for(e, d);
    web_reports(e).into_iter().find(|x| x.0 == id).map(|x| x.1)
}

fn get(e: &mut Engine, url: &str) -> Option<Result<WebResp, WebFail>> {
    fetch_from(e, PC1, req(1, "GET", url, "", ""), SEC)
}

/// status and body of an answer, the error kind of a failure
fn shown(r: Option<Result<WebResp, WebFail>>) -> String {
    match r {
        Some(Ok(a)) => format!("{} {}", a.status, String::from_utf8_lossy(&a.body)),
        Some(Err(f)) => format!("{:?}", f.kind),
        None => s("nichts"),
    }
}

fn kind(r: &Option<Result<WebResp, WebFail>>) -> Option<WebErr> {
    r.as_ref().and_then(|x| x.as_ref().err().map(|f| f.kind))
}

fn body_has(r: &Option<Result<WebResp, WebFail>>, what: &str) -> bool {
    r.as_ref().and_then(|x| x.as_ref().ok()).is_some_and(|a| String::from_utf8_lossy(&a.body).contains(what))
}

fn ctype(r: &Option<Result<WebResp, WebFail>>) -> String {
    r.as_ref().and_then(|x| x.as_ref().ok()).and_then(|a| a.headers.get("content-type")).map(String::from).unwrap_or_default()
}

// ---------------------------------------------------------------- the URLs

fn uris(t: &mut T) {
    // the parts
    let u = Uri::parse("HTTP://anna:geheim@web.lab:8080/a/b.html?x=1&y=2#teil");
    t.eq("scheme in lower case", u.scheme.as_deref(), Some("http"));
    t.eq("authority", u.authority.as_deref(), Some("anna:geheim@web.lab:8080"));
    t.eq("user info", u.userinfo(), Some("anna:geheim"));
    t.eq("host", u.host(), Some("web.lab"));
    t.eq("port", u.port(), Some(8080));
    t.eq("path", u.path.as_str(), "/a/b.html");
    t.eq("query", u.query.as_deref(), Some("x=1&y=2"));
    t.eq("fragment", u.fragment.as_deref(), Some("teil"));
    t.eq("target", u.target(), s("/a/b.html?x=1&y=2"));
    t.eq("written again", u.text(), s("http://anna:geheim@web.lab:8080/a/b.html?x=1&y=2#teil"));
    let six = Uri::parse("http://[2001:db8::1]:8080/");
    t.eq("IPv6 host", six.host(), Some("2001:db8::1"));
    t.eq("IPv6 port", six.port(), Some(8080));
    t.eq("no port", Uri::parse("http://[2001:db8::1]/").port(), None);
    t.eq("a port too large", Uri::parse("http://h:70000/").port(), None);
    t.eq("no path", Uri::parse("http://h").target(), s("/"));
    t.eq("only a query", Uri::parse("?a=b").query.as_deref(), Some("a=b"));
    t.eq("a colon after a slash is no scheme", Uri::parse("a/b:c").scheme, None);
    t.eq("a digit cannot start a scheme", Uri::parse("10.0.0.1:80/x").scheme, None);
    // RFC 3986 5.4.1 and 5.4.2
    let base = Uri::parse("http://a/b/c/d;p?q");
    let cases: [(&str, &str); 42] = [
        ("g:h", "g:h"),
        ("g", "http://a/b/c/g"),
        ("./g", "http://a/b/c/g"),
        ("g/", "http://a/b/c/g/"),
        ("/g", "http://a/g"),
        ("//g", "http://g"),
        ("?y", "http://a/b/c/d;p?y"),
        ("g?y", "http://a/b/c/g?y"),
        ("#s", "http://a/b/c/d;p?q#s"),
        ("g#s", "http://a/b/c/g#s"),
        ("g?y#s", "http://a/b/c/g?y#s"),
        (";x", "http://a/b/c/;x"),
        ("g;x", "http://a/b/c/g;x"),
        ("g;x?y#s", "http://a/b/c/g;x?y#s"),
        ("", "http://a/b/c/d;p?q"),
        (".", "http://a/b/c/"),
        ("./", "http://a/b/c/"),
        ("..", "http://a/b/"),
        ("../", "http://a/b/"),
        ("../g", "http://a/b/g"),
        ("../..", "http://a/"),
        ("../../", "http://a/"),
        ("../../g", "http://a/g"),
        // the abnormal examples
        ("../../../g", "http://a/g"),
        ("../../../../g", "http://a/g"),
        ("/./g", "http://a/g"),
        ("/../g", "http://a/g"),
        ("g.", "http://a/b/c/g."),
        (".g", "http://a/b/c/.g"),
        ("g..", "http://a/b/c/g.."),
        ("..g", "http://a/b/c/..g"),
        ("./../g", "http://a/b/g"),
        ("./g/.", "http://a/b/c/g/"),
        ("g/./h", "http://a/b/c/g/h"),
        ("g/../h", "http://a/b/c/h"),
        ("g;x=1/./y", "http://a/b/c/g;x=1/y"),
        ("g;x=1/../y", "http://a/b/c/y"),
        ("g?y/./x", "http://a/b/c/g?y/./x"),
        ("g?y/../x", "http://a/b/c/g?y/../x"),
        ("g#s/./x", "http://a/b/c/g#s/./x"),
        ("g#s/../x", "http://a/b/c/g#s/../x"),
        ("http:g", "http:g"),
    ];
    for (r, want) in cases {
        t.eq(&format!("RFC 3986: {}", r), base.join(r).text(), s(want));
    }
    // a base without a path, dot segments, what a page does
    t.eq("a base without a path", Uri::parse("http://a").join("g").text(), s("http://a/g"));
    t.eq("dot segments", remove_dot_segments("/a/b/c/./../../g"), s("/a/g"));
    t.eq("dot segments, RFC's second", remove_dot_segments("mid/content=5/../6"), s("mid/6"));
    t.eq("above the root", remove_dot_segments("/../../etc/hosts"), s("/etc/hosts"));
    t.eq("the root stays", remove_dot_segments("/.."), s("/"));
    t.eq("a page's relative link", Uri::parse("http://web.lab/brot/index.html").join("../index.html").text(), s("http://web.lab/index.html"));
    t.eq("a page's other port", Uri::parse("http://web.lab/").join("http://web.lab:8080/").text(), s("http://web.lab:8080/"));
    // percent encoding
    t.eq("characters a URI cannot hold", Uri::parse("http://h/ä ö?q=a b#c d").encoded().text(), s("http://h/%C3%A4%20%C3%B6?q=a%20b#c%20d"));
    t.eq("escapes stay", Uri::parse("http://h/a%20b?x=%C3%A4").encoded().text(), s("http://h/a%20b?x=%C3%A4"));
    t.eq("a lone percent", Uri::parse("http://h/50%").encoded().text(), s("http://h/50%25"));
    t.eq("reserved characters stay", Uri::parse("http://h/a;b=c,d?e=f&g=h").encoded().text(), s("http://h/a;b=c,d?e=f&g=h"));
    t.eq("percent_encode", uri::percent_encode(b"a b/c", b"/"), s("a%20b/c"));
    t.eq("percent_decode", uri::percent_decode("a%20b%zz%4"), Vec::from(&b"a b%zz%4"[..]));
    t.eq("percent_text", uri::percent_text("K%C3%A4se"), s("Käse"));
    // form encoding (HTML: + for a space, little left alone)
    let pairs = alloc::vec![(s("ware"), s("Käse & Brot")), (s("a=b"), s("c+d")), (s("leer"), s(""))];
    let enc = uri::form_encode(&pairs);
    t.eq("form_encode", enc.clone(), s("ware=K%C3%A4se+%26+Brot&a%3Db=c%2Bd&leer="));
    t.eq("form_decode", uri::form_decode(&enc), pairs);
    t.eq("form_decode without =", uri::form_decode("a&b=1&&"), alloc::vec![(s("a"), s("")), (s("b"), s("1"))]);
    // the address line
    t.eq("address: no scheme", address(" web.lab/x "), s("http://web.lab/x"));
    t.eq("address: a port", address("10.0.0.1:8080/a b"), s("http://10.0.0.1:8080/a%20b"));
    t.eq("address: a scheme", address("http://web.lab"), s("http://web.lab/"));
    t.eq("address: no path", address("web.lab:8080"), s("http://web.lab:8080/"));
    t.eq("address: only a query", address("web.lab?a=1"), s("http://web.lab/?a=1"));
    t.eq("address: https", address("HTTPS://web.lab/"), s("https://web.lab/"));
    t.eq("address: empty", address("   "), s(""));
    t.eq("address: mailto", address("mailto:a@b.lab"), s("mailto:a@b.lab"));
}

// ---------------------------------------------------------------- the error kinds

fn errors(t: &mut T) {
    let mut e = engine();
    let r = get(&mut e, "http://10.0.0.1:81/");
    t.eq("refused", kind(&r), Some(WebErr::Refused));
    t.eq("refused: the URL", r.and_then(|x| x.err()).map(|f| f.url), Some(s("http://10.0.0.1:81/")));
    let r = get(&mut e, "http://gibtsnicht.lab/");
    t.eq("name unknown", kind(&r), Some(WebErr::NoName));
    t.check("name unknown: says why", r.and_then(|x| x.err()).is_some_and(|f| f.msg.text().contains("Name nicht aufgelöst")));
    // no name server at all (DNS1 asks nobody)
    t.eq("no name server", kind(&fetch_from(&mut e, DNS1, req(2, "GET", "http://web.lab/", "", ""), SEC)), Some(WebErr::NoName));
    // PC1 has no gateway
    t.eq("no route", kind(&get(&mut e, "http://192.168.7.7/")), Some(WebErr::NoRoute));
    // a cable pulled: the link is down, so is the route
    let mut c = engine();
    let link = project().links.iter().find(|l| l.a.dev == PC1 || l.b.dev == PC1).map_or(0, |l| l.id);
    t.check("the cable goes", c.apply(Action::Disconnect(link)).is_ok());
    run_for(&mut c, SEC);
    t.eq("network down", kind(&get(&mut c, "http://10.0.0.1/")), Some(WebErr::NoRoute));
    // nobody answers: the handler never does, 30 s of virtual time
    let before = e.now();
    let r = fetch_from(&mut e, PC1, req(3, "GET", "http://web.lab:8080/langsam", "", ""), 29 * SEC);
    t.check("timeout: nothing after 29 s", r.is_none());
    run_for(&mut e, 2 * SEC);
    let late = web_reports(&mut e).into_iter().find(|x| x.0 == 3).map(|x| x.1);
    t.eq("timeout", kind(&late), Some(WebErr::Timeout));
    t.check("timeout: after 30 s", e.now() - before >= 30 * SEC);
    // an address nobody has on the LAN: ARP gives up
    let far = fetch_from(&mut e, PC1, req(4, "GET", "http://10.0.0.99/", "", ""), 40 * SEC);
    t.check("nobody at the address", matches!(kind(&far), Some(WebErr::NoRoute | WebErr::Timeout)));
    // not http
    // https goes to port 443, where nobody listens here (a server that speaks TLS: group webdev)
    t.eq("https: port 443", kind(&get(&mut e, "https://web.lab/")), Some(WebErr::Refused));
    // the web server of port 80 speaks no TLS: its answer is no record
    let r = get(&mut e, "https://web.lab:80/");
    t.eq("https: a port without TLS", kind(&r), Some(WebErr::Tls));
    t.check("https: says why", r.and_then(|x| x.err()).is_some_and(|f| f.msg.text().contains("protocol_version")));
    t.eq("another scheme", kind(&get(&mut e, "ftp://web.lab/")), Some(WebErr::BadUrl));
    t.eq("no host", kind(&get(&mut e, "http:///a")), Some(WebErr::BadUrl));
    t.eq("a bad port", kind(&get(&mut e, "http://web.lab:99999/")), Some(WebErr::BadUrl));
    // the kinds of the client's failures
    use jfn_sim::http::Fail;
    use jfn_sim::tcp::TcpEnd;
    t.eq("reset: broken", kind_of(&Fail::Reset(TcpEnd::Reset)), WebErr::Broken);
    t.eq("empty: broken", kind_of(&Fail::Empty), WebErr::Broken);
    t.eq("truncated: broken", kind_of(&Fail::Truncated { got: 1, of: Some(2) }), WebErr::Broken);
    t.eq("connect, unreachable: no route", kind_of(&Fail::Connect(TcpEnd::Unreachable)), WebErr::NoRoute);
    t.eq("connect, timeout", kind_of(&Fail::Connect(TcpEnd::Timeout)), WebErr::Timeout);
    t.eq("connect timeout", kind_of(&Fail::ConnectTimeout), WebErr::Timeout);
    // the pages of the failures are ours and name the address
    let (title, html) = error_page(WebErr::Refused, "http://web.lab:81/", "keine Verbindung");
    t.eq("error page: its title", title, s("Seite nicht erreichbar"));
    t.check("error page: the port", html.contains("Port 81"));
    t.check("error page: the address", html.contains("http://web.lab:81/"));
    t.check("error page: why", html.contains("keine Verbindung"));
    let (title, _) = error_page(WebErr::TooManyRedirects, "http://a/", "");
    t.eq("error page: redirects", title, s("Zu viele Weiterleitungen"));
    t.check("an address is escaped on the page", error_page(WebErr::BadUrl, "x:<b>", "").1.contains("x:&lt;b&gt;"));
    t.eq("esc", esc("<a href=\"x\">&"), s("&lt;a href=&quot;x&quot;&gt;&amp;"));
}

// ---------------------------------------------------------------- redirects

fn redirects(t: &mut T) {
    let b = || Vec::from(&b"a=1"[..]);
    // the rules
    t.eq("301 POST -> GET", redirected(301, s("POST"), b()), (s("GET"), Vec::new()));
    t.eq("302 POST -> GET", redirected(302, s("POST"), b()), (s("GET"), Vec::new()));
    t.eq("303 POST -> GET", redirected(303, s("POST"), b()), (s("GET"), Vec::new()));
    t.eq("303 PUT -> GET", redirected(303, s("PUT"), b()), (s("GET"), Vec::new()));
    t.eq("303 HEAD stays", redirected(303, s("HEAD"), Vec::new()), (s("HEAD"), Vec::new()));
    t.eq("301 PUT stays", redirected(301, s("PUT"), b()), (s("PUT"), b()));
    t.eq("307 POST stays", redirected(307, s("POST"), b()), (s("POST"), b()));
    t.eq("308 POST stays", redirected(308, s("POST"), b()), (s("POST"), b()));
    // over the network: what /echo says it got
    let mut e = engine();
    let form = "application/x-www-form-urlencoded";
    for (path, want) in [("/moved", "200 GET \n"), ("/found", "200 GET \n"), ("/see", "200 GET \n"), ("/temp", "200 POST a=1\n"), ("/perm", "200 POST a=1\n")] {
        let url = format!("http://web.lab:8080{}", path);
        let r = fetch_from(&mut e, PC1, req(5, "POST", &url, form, "a=1"), SEC);
        t.eq(&format!("POST to {}", path), shown(r.clone()), s(want));
        let a = r.and_then(|x| x.ok());
        t.eq(&format!("{}: the final URL", path), a.as_ref().map(|x| x.url.clone()), Some(s("http://web.lab:8080/echo")));
        t.eq(&format!("{}: one redirect", path), a.map(|x| x.redirects), Some(1));
    }
    let r = fetch_from(&mut e, PC1, req(6, "GET", "http://web.lab:8080/weiter", "", ""), SEC);
    t.check("a relative Location with a query", body_has(&r, "Gesucht wurde: weitergeleitet"));
    t.eq("a redirect's final URL", r.and_then(|x| x.ok()).map(|a| a.url), Some(s("http://web.lab:8080/suche?q=weitergeleitet")));
    t.check("the redirect is a note of the device", e.trace.events().any(|x| x.dev == PC1 && matches!(&x.note, jfn_sim::Note::WebRedirect { status: 302, .. })));
    // the request's fragment stays
    let r = fetch_from(&mut e, PC1, req(7, "GET", "http://web.lab:8080/weiter#unten", "", ""), SEC);
    t.eq("the fragment stays", r.and_then(|x| x.ok()).map(|a| a.url), Some(s("http://web.lab:8080/suche?q=weitergeleitet#unten")));
    // the limit: the 11th redirect is too many
    let v0 = e.web_client(PC1).map_or(0, |v| v.redirects);
    let r = fetch_from(&mut e, PC1, req(8, "GET", "http://web.lab:8080/schleife", "", ""), SEC);
    t.eq("too many redirects", kind(&r), Some(WebErr::TooManyRedirects));
    t.eq("followed up to the limit", e.web_client(PC1).map_or(0, |v| v.redirects) - v0, MAX_REDIRECTS as u64);
    // curl -L goes the same way; -X stays the word of every request, the data goes all the same
    let curl = |e: &mut Engine, args: &str| {
        let _ = e.take_reports();
        let _ = e.cli_line(PC1, &format!("curl -s -L {}", args));
        run_for(e, SEC);
        let mut o = String::new();
        for r in e.take_reports() {
            if let Report::CliOut { dev: PC1, bytes, .. } = r {
                o.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        o
    };
    for (path, want) in [("/moved", "GET \n"), ("/found", "GET \n"), ("/see", "GET \n"), ("/temp", "POST a=1\n"), ("/perm", "POST a=1\n")] {
        t.eq(&format!("curl -L -d to {}", path), curl(&mut e, &format!("-d a=1 http://web.lab:8080{}", path)), s(want));
    }
    t.eq("curl -X POST -d: the word stays, the data goes", curl(&mut e, "-X POST -d a=1 http://web.lab:8080/found"), s("POST \n"));
    t.eq("curl -X PUT -d: the same", curl(&mut e, "-X PUT -d a=1 http://web.lab:8080/moved"), s("PUT \n"));
    t.eq("curl: 307 keeps the -X word and the data", curl(&mut e, "-X PUT -d a=1 http://web.lab:8080/temp"), s("PUT a=1\n"));
    t.eq("curl: the next URL posts again", curl(&mut e, "-d a=1 http://web.lab:8080/found http://web.lab:8080/echo"), s("GET \nPOST a=1\n"));
    let v = curl(&mut e, "-v -d a=1 http://web.lab:8080/see");
    t.check("curl -v: POST, then GET", v.contains("> POST /see HTTP/1.1\r\n") && v.contains("> GET /echo HTTP/1.1\r\n"));
    t.eq("curl -v: data only on the first request", v.matches("> Content-Length: 3\r\n").count(), 1);
    t.eq("curl -v: its type too", v.matches("> Content-Type: application/x-www-form-urlencoded\r\n").count(), 1);
    t.check("curl -v says so", v.contains("* The redirect drops the data of -d\n"));
    let v = curl(&mut e, "-v -d a=1 http://web.lab:8080/temp");
    t.check("curl -v: 307 keeps the data", !v.contains("drops the data") && v.matches("> Content-Length: 3\r\n").count() == 2);
    let v = curl(&mut e, "-v -d a=1 -H 'Content-Type: text/plain' http://web.lab:8080/found");
    t.eq("curl -v: a type of -H stays on every request", v.matches("> Content-Type: text/plain\r\n").count(), 2);
    // a redirect of the static server: a folder without its slash
    let r = get(&mut e, "http://web.lab/brot?x=1");
    t.eq("a folder gets its slash", r.as_ref().and_then(|x| x.as_ref().ok()).map(|a| a.url.clone()), Some(s("http://web.lab/brot/?x=1")));
    t.check("the folder's index.html", body_has(&r, "Unser Brot"));
}

// ---------------------------------------------------------------- static files and types

fn statics(t: &mut T) {
    let mut e = engine();
    let r = get(&mut e, "http://web.lab/");
    t.check("/: index.html", body_has(&r, "<title>Bäckerei Krume</title>"));
    t.eq("/: the type", ctype(&r), s("text/html; charset=utf-8"));
    t.eq("/brot/: its index", body_has(&get(&mut e, "http://web.lab/brot/"), "Roggenbrot"), true);
    let r = get(&mut e, "http://web.lab/notiz.txt");
    t.check("a text file", body_has(&r, "Mehl     25 kg"));
    t.eq("its type", ctype(&r), s("text/plain; charset=utf-8"));
    t.eq("Markdown", ctype(&get(&mut e, "http://web.lab/hilfe.md")), s("text/markdown; charset=utf-8"));
    let r = get(&mut e, "http://web.lab/logo.svg");
    t.eq("a picture", ctype(&r), s("image/svg+xml"));
    t.check("the picture's bytes", body_has(&r, "<svg"));
    let r = get(&mut e, "http://web.lab/daten.bin");
    t.eq("bytes", ctype(&r), s("application/octet-stream"));
    t.eq("all of them", r.and_then(|x| x.ok()).map(|a| a.body.len()), Some(64));
    t.eq("not there", shown(get(&mut e, "http://web.lab/fehlt.html")), s("404 404 Not Found\n"));
    // the folder holds: dot segments stay inside, encoded or not
    t.eq("../ stays inside", shown(get(&mut e, "http://web.lab/../../root/app/laden.jfs")), s("404 404 Not Found\n"));
    t.eq("%2e%2e stays inside", shown(get(&mut e, "http://web.lab/%2e%2e/%2e%2e/root/app/laden.jfs")), s("404 404 Not Found\n"));
    t.eq("encoded names", body_has(&get(&mut e, "http://web.lab/noti%7A.txt"), "Mehl"), true);
    // methods
    let r = fetch_from(&mut e, PC1, req(2, "POST", "http://web.lab/index.html", "text/plain", "x"), SEC);
    t.eq("POST on a file", r.as_ref().and_then(|x| x.as_ref().ok()).map(|a| a.status), Some(405));
    t.eq("Allow", r.as_ref().and_then(|x| x.as_ref().ok()).and_then(|a| a.headers.get("allow").map(String::from)), Some(s("GET, HEAD")));
    let r = fetch_from(&mut e, PC1, req(3, "HEAD", "http://web.lab/index.html", "", ""), SEC);
    t.eq("HEAD: no body", r.as_ref().and_then(|x| x.as_ref().ok()).map(|a| (a.status, a.body.len())), Some((200, 0)));
    // a saved page shows at once
    t.check("a page changes", e.apply(Action::Files { dev: WEB1, files: project().devices.iter().find(|d| d.id == WEB1).map(|d| d.files.iter().cloned().map(|mut f| { if f.path == "/var/www/notiz.txt" { f.data = Vec::from(&b"neu\n"[..]); } f }).collect()).unwrap_or_default() }).is_ok());
    t.eq("it shows", shown(get(&mut e, "http://web.lab/notiz.txt")), s("200 neu\n"));
    // the types by name
    for (name, want) in [("a.HTML", "text/html; charset=utf-8"), ("a.htm", "text/html; charset=utf-8"), ("b.jpeg", "image/jpeg"), ("c.png", "image/png"), ("d.css", "text/css; charset=utf-8"), ("e.json", "application/json"), ("f", "application/octet-stream"), ("g.tar.gz", "application/octet-stream"), (".h/i", "application/octet-stream")] {
        t.eq(&format!("mime_of {}", name), mime_of(name), want);
    }
    // routes come first, a server without "root" says what it said before
    let mut p = project();
    if let Some(d) = p.device_mut(WEB1) {
        d.services[0].config = agent_core::json::parse(br#"{"port": 80, "root": "/var/www", "routes": [{"path": "/notiz.txt", "body": "von der Route\n"}]}"#).unwrap_or(agent_core::jmap::Value::Null);
    }
    let mut r2 = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut r2, 2 * SEC);
    t.eq("a route comes first", shown(get(&mut r2, "http://web.lab/notiz.txt")), s("200 von der Route\n"));
    t.check("the files stay", body_has(&get(&mut r2, "http://web.lab/hilfe.md"), "# Hilfe"));
    if let Some(d) = p.device_mut(WEB1) {
        d.services[0].config = agent_core::json::parse(br#"{"port": 80}"#).unwrap_or(agent_core::jmap::Value::Null);
    }
    let mut r3 = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut r3, 2 * SEC);
    t.eq("no root, no routes: as before", shown(get(&mut r3, "http://web.lab/")), s("200 Hallo von WEB1\n"));
    if let Some(d) = p.device_mut(WEB1) {
        d.services[0].config = agent_core::json::parse(br#"{"root": "var/www"}"#).unwrap_or(agent_core::jmap::Value::Null);
    }
    t.check("a relative root is refused", Engine::from_project(&p).is_err() || !Engine::from_project(&p).is_ok_and(|e| e.services(WEB1).iter().any(|v| v.name == "www")));
    // the views by type
    t.eq("html", classify("text/html; charset=utf-8", b"<p>"), Kind::Html);
    t.eq("HTML in capitals", classify("TEXT/HTML", b""), Kind::Html);
    t.eq("markdown", classify("text/markdown", b"# a"), Kind::Markdown);
    t.eq("plain", classify("text/plain", b"a"), Kind::Text);
    t.eq("other text", classify("text/csv", b"a,b"), Kind::Text);
    t.eq("json", classify("application/json", b"{}"), Kind::Text);
    t.eq("an image", classify("image/png", b"\x89PNG"), Kind::Image);
    t.eq("svg is an image", classify("image/svg+xml", b"<svg"), Kind::Image);
    t.eq("no type, text", classify("", b"Hallo\n"), Kind::Text);
    t.eq("no type, bytes", classify("", b"\x00\x01"), Kind::Binary);
    t.eq("unknown, text", classify("application/x-foo", b"Hallo"), Kind::Text);
    t.eq("unknown, bytes", classify("application/octet-stream", &[0xFF, 0xFE, 0]), Kind::Binary);
    // data: addresses
    t.eq("data: base64", data_url("data:image/png;base64,SGFsbG8="), Some(Vec::from(&b"Hallo"[..])));
    t.eq("data: text", data_url("data:,a%20b"), Some(Vec::from(&b"a b"[..])));
    t.eq("no data:", data_url("http://a/"), None);
}

// ---------------------------------------------------------------- forms

fn forms(t: &mut T) {
    let sub = |action: &str, method: &str| MdSubmit { form: 0, action: s(action), method: s(method), fields: alloc::vec![(s("q"), s("Käse & Brot")), (s("n"), s("2"))], button: None };
    t.eq("GET: the query", form_nav("http://web.lab:8080/", &sub("/suche", "get")), Nav::get("http://web.lab:8080/suche?q=K%C3%A4se+%26+Brot&n=2"));
    t.eq("GET: the action's own query goes", form_nav("http://h/a/b", &sub("c?x=1#y", "get")).url, s("http://h/a/c?q=K%C3%A4se+%26+Brot&n=2"));
    t.eq("no action: the page", form_nav("http://h/a/b?old=1", &sub("", "get")).url, s("http://h/a/b?q=K%C3%A4se+%26+Brot&n=2"));
    let p = form_nav("http://h/a/b", &sub("/k", "POST"));
    t.eq("POST: the method", p.method, s("POST"));
    t.eq("POST: the action", p.url, s("http://h/k"));
    t.eq("POST: the body", String::from_utf8_lossy(&p.body).into_owned(), s("q=K%C3%A4se+%26+Brot&n=2"));
    t.eq("POST: its type", p.ctype, s("application/x-www-form-urlencoded"));
    t.eq("an unknown method is GET", form_nav("http://h/", &sub("/k", "dialog")).method, s("GET"));
    // a script reads both
    let mut e = engine();
    let g = form_nav("http://web.lab:8080/", &MdSubmit { action: s("/suche"), method: s("get"), fields: alloc::vec![(s("q"), s("Käse & <Brot>"))], ..MdSubmit::default() });
    let r = fetch_from(&mut e, PC1, req(1, &g.method, &g.url, "", ""), SEC);
    t.check("the script's req.query", body_has(&r, "Gesucht wurde: Käse &amp; &lt;Brot>"));
    let fields = alloc::vec![(s("ware"), s("Käse & Brot")), (s("menge"), s("3"))];
    let n = form_nav("http://web.lab:8080/", &MdSubmit { action: s("/bestellen"), method: s("post"), fields, ..MdSubmit::default() });
    let body = String::from_utf8_lossy(&n.body).into_owned();
    let r = fetch_from(&mut e, PC1, req(2, &n.method, &n.url, &n.ctype, &body), SEC);
    t.check("the script's req.form", body_has(&r, "Bestellung 1: 3 mal Käse &amp; Brot"));
    let r = fetch_from(&mut e, PC1, req(3, "GET", "http://web.lab:8080/bestellen", "", ""), SEC);
    t.eq("the script tells the methods apart", r.and_then(|x| x.ok()).map(|a| a.status), Some(405));
    // another type of body gives an empty form
    let r = fetch_from(&mut e, PC1, req(4, "POST", "http://web.lab:8080/bestellen", "text/plain", "ware=Brot&menge=2"), SEC);
    t.check("req.form only for a form's type", body_has(&r, "Bestellung 2:  mal "));
    // the other answers
    t.eq("500 with a page", get(&mut e, "http://web.lab:8080/kaputt").and_then(|x| x.ok()).map(|a| a.status), Some(500));
    t.eq("503 without one", shown(get(&mut e, "http://web.lab:8080/fehler")), s("503 503 Service Unavailable\n"));
    t.eq("204", get(&mut e, "http://web.lab:8080/leer").and_then(|x| x.ok()).map(|a| (a.status, a.body.len())), Some((204, 0)));
}

// ---------------------------------------------------------------- the history, fragments

fn history(t: &mut T) {
    let mut h: Hist<u32> = Hist::new();
    t.check("nothing yet", h.cur().is_none() && !h.can_back() && !h.can_forward());
    h.push(1);
    h.push(2);
    h.push(3);
    t.eq("the last", h.cur().copied(), Some(3));
    t.check("back", h.back() && h.cur() == Some(&2));
    t.check("back again", h.back() && h.cur() == Some(&1));
    t.check("no further", !h.back() && h.cur() == Some(&1));
    t.check("forward", h.forward() && h.cur() == Some(&2));
    h.push(9);
    t.eq("a new page: what lay ahead goes", (h.len(), h.cur().copied(), h.can_forward()), (3, Some(9), false));
    h.replace(10);
    t.eq("reload: its place", (h.len(), h.cur().copied()), (3, Some(10)));
    for i in 0..100 {
        h.push(i);
    }
    t.eq("the limit", h.len(), HIST_MAX);
    t.eq("still the newest", h.cur().copied(), Some(99));
    let mut back = 0;
    while h.back() {
        back += 1;
    }
    t.eq("back through all of it", back, HIST_MAX - 1);
    t.eq("the oldest kept", h.cur().copied(), Some(100 - HIST_MAX as u32));
    let mut empty: Hist<u32> = Hist::new();
    empty.replace(5);
    t.eq("replace without an entry", empty.cur().copied(), Some(5));
    // fragments: an id, a name, a block around, a container, Markdown's headings
    let d = MdDoc::html("<p>a</p><h2 id=\"zwei\">Zwei</h2><p>b <a id=\"innen\">c</a></p><div id=\"kasten\"><p>d</p></div><p><a name=\"alt\">e</a></p>");
    let text = |i: Option<usize>| i.and_then(|i| d.blocks().get(i)).map(|b| String::from(b.text.trim()));
    t.eq("fragment: an id", text(block_of(&d, "zwei")), Some(s("Zwei")));
    t.eq("fragment: inside a paragraph", text(block_of(&d, "innen")), Some(s("b c")));
    t.eq("fragment: a container", text(block_of(&d, "kasten")), Some(s("d")));
    t.eq("fragment: a name", text(block_of(&d, "alt")), Some(s("e")));
    t.eq("fragment: unknown", block_of(&d, "nichts"), None);
    t.eq("fragment: empty is the top", block_of(&d, ""), Some(0));
    let m = MdDoc::markdown("# Erster Teil\n\ntext\n\n## Zweiter Teil!\n\nmehr\n");
    t.eq("fragment: a heading of Markdown", block_of(&m, "zweiter-teil").and_then(|i| m.blocks().get(i)).map(|b| String::from(b.text.trim())), Some(s("Zweiter Teil!")));
    let k = MdDoc::html("<h2 id=\"%C3%BC\">a</h2><h2 id=\"ü\">b</h2>");
    t.eq("fragment: percent-encoded", block_of(&k, "%C3%BC").and_then(|i| k.blocks().get(i)).map(|b| String::from(b.text.trim())), Some(s("b")));
}

// ---------------------------------------------------------------- what the service does

fn service(t: &mut T) {
    let mut e = engine();
    t.eq("no service before the first request", e.web_client(PC1), None);
    let r = get(&mut e, "http://web.lab/");
    t.check("the first request makes it", r.is_some() && e.web_client(PC1).is_some());
    t.check("hidden among the services", !e.services(PC1).iter().any(|v| v.name == CLIENT_SVC));
    // the request went over the network: the server counted it, the trace has its frames
    t.check("the server's numbers", e.metric(WEB1, "www", "requests").is_some_and(|n| n >= 1));
    t.check("the frames of DNS and HTTP", e.trace.hops().any(|h| h.mask & netpkt::decode::P_DNS != 0) && e.trace.hops().any(|h| h.mask & netpkt::decode::P_HTTP != 0));
    let conns = e.web_client(PC1).map_or(0, |v| v.requests);
    t.eq("it counted it", conns, 1);
    // cancelling: no report, the connection goes, the next request works
    let _ = e.take_reports();
    t.check("a request that hangs", e.apply(Action::WebFetch { dev: PC1, req: req(5, "GET", "http://web.lab:8080/langsam", "", "") }).is_ok());
    run_for(&mut e, SEC);
    t.eq("it runs", e.web_client(PC1).map(|v| v.active), Some(alloc::vec![5]));
    t.check("cancelled", e.apply(Action::WebCancel { dev: PC1, id: 5 }).is_ok());
    run_for(&mut e, 40 * SEC);
    t.eq("no report for it", web_reports(&mut e).len(), 0);
    t.eq("it is gone", e.web_client(PC1).map(|v| v.active), Some(Vec::new()));
    t.check("cancelling what is gone is nothing", e.apply(Action::WebCancel { dev: PC1, id: 5 }).is_ok());
    t.eq("the next request works", body_has(&get(&mut e, "http://web.lab/"), "Krume"), true);
    // a device without the service: cancelling does nothing
    t.check("no service: nothing to cancel", e.apply(Action::WebCancel { dev: DNS1, id: 9 }).is_ok());
    // several at once, in any order of answers: each comes with its id
    let _ = e.take_reports();
    for (id, path) in [(11u64, "/index.html"), (12, "/notiz.txt"), (13, "/logo.svg"), (14, "/hilfe.md")] {
        let _ = e.apply(Action::WebFetch { dev: PC1, req: req(id, "GET", &format!("http://web.lab{}", path), "", "") });
    }
    run_for(&mut e, SEC);
    let mut ids: Vec<u64> = web_reports(&mut e).into_iter().filter(|x| x.1.is_ok()).map(|x| x.0).collect();
    ids.sort_unstable();
    t.eq("four at once", ids, alloc::vec![11, 12, 13, 14]);
    // more than a host takes connections: the others wait for a free one
    let _ = e.take_reports();
    for id in 40..60u64 {
        let _ = e.apply(Action::WebFetch { dev: PC1, req: req(id, "GET", "http://web.lab/notiz.txt", "", "") });
    }
    run_for(&mut e, SEC);
    let done = web_reports(&mut e);
    t.eq("twenty at once", (done.len(), done.iter().all(|x| x.1.is_ok())), (20, true));
    // the power goes off under a request; a device that is off refuses
    let _ = e.take_reports();
    let _ = e.apply(Action::WebFetch { dev: PC1, req: req(20, "GET", "http://web.lab:8080/langsam", "", "") });
    run_for(&mut e, SEC);
    t.check("off", e.apply(Action::Power { dev: PC1, on: false }).is_ok());
    t.eq("stopped", web_reports(&mut e).into_iter().find(|x| x.0 == 20).and_then(|x| x.1.err()).map(|f| f.kind), Some(WebErr::Stopped));
    t.check("a device that is off refuses", e.apply(Action::WebFetch { dev: PC1, req: req(21, "GET", "http://web.lab/", "", "") }).is_err());
    t.check("on again", e.apply(Action::Power { dev: PC1, on: true }).is_ok());
    run_for(&mut e, 3 * SEC);
    t.eq("it works again", body_has(&fetch_from(&mut e, PC1, req(22, "GET", "http://web.lab/", "", ""), 2 * SEC), "Krume"), true);
    // kept in a keyframe: what it holds counts
    let f0 = jfn_sim::keyframe::footprint(&e);
    let _ = e.apply(Action::WebFetch { dev: PC1, req: req(30, "POST", "http://web.lab:8080/langsam", "text/plain", &"x".repeat(100_000)) });
    t.check("the footprint counts a request", jfn_sim::keyframe::footprint(&e) >= f0 + 100_000);
}

// one run of the browser's requests -> what it reported and the state at its end
fn a_run() -> (Vec<(u64, Result<WebResp, WebFail>)>, u64, usize) {
    let mut e = engine();
    let _ = e.take_reports();
    let mut out = Vec::new();
    for (id, m, url, ct, body) in [
        (1u64, "GET", "http://web.lab/", "", ""),
        (2, "GET", "http://web.lab/logo.svg", "", ""),
        (3, "GET", "http://web.lab:8080/weiter", "", ""),
        (4, "POST", "http://web.lab:8080/bestellen", "application/x-www-form-urlencoded", "ware=Brot&menge=2"),
        (5, "GET", "http://gibtsnicht.lab/", "", ""),
        (6, "GET", "http://10.0.0.1:81/", "", ""),
    ] {
        let _ = e.apply(Action::WebFetch { dev: PC1, req: req(id, m, url, ct, body) });
        run_for(&mut e, 500 * MS);
        out.extend(web_reports(&mut e));
    }
    (out, digest(&e), e.trace.hops().len())
}

fn determinism(t: &mut T) {
    let (a, da, ha) = a_run();
    let (b, db, hb) = a_run();
    t.eq("all six reported", a.len(), 6);
    t.check("two runs report the same, on the nanosecond", a == b);
    t.eq("the same state", da, db);
    t.eq("the same frames", ha, hb);
    // a recorded run replays: the commands are inputs, a replay ends in the same state
    let mut d = Inline::new(Engine::from_project(&project()).unwrap_or_else(|_| Engine::new(0)));
    d.rewind_on(Budget::default());
    let _ = d.send(Cmd::RunFor(2 * SEC), 0);
    let _ = d.send(Cmd::Apply(Action::WebFetch { dev: PC1, req: req(1, "GET", "http://web.lab/", "", "") }), 0);
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let _ = d.send(Cmd::Apply(Action::WebFetch { dev: PC1, req: req(2, "GET", "http://web.lab:8080/langsam", "", "") }), 0);
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let _ = d.send(Cmd::Apply(Action::WebCancel { dev: PC1, id: 2 }), 0);
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let live = digest(&d.eng);
    let reports = d.reports().iter().filter(|r| matches!(r, Report::Web { .. })).count();
    t.eq("one answer, the other given up", reports, 1);
    let pos = d.pos();
    let replayed = d.restore_pos(pos, None).and_then(|mut r| {
        let rw = d.rewind()?;
        r.run(rw, usize::MAX).then(|| r.finish())
    });
    t.eq("a replay ends in the same state", replayed.map(|p| digest(&p.eng)), Some(live));
}

pub(crate) fn run(t: &mut T) {
    uris(t);
    errors(t);
    redirects(t);
    statics(t);
    forms(t);
    history(t);
    service(t);
    determinism(t);
}
