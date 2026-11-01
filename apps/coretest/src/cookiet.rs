// cookiet -- checks of netpkt::cookie (group cookie): what a Set-Cookie header says with every
// attribute and what cannot be read, the tolerant date of RFC 6265 5.1.1 with the examples of real
// servers, domain-match and path-match with the RFC's rules, the storage rules (host-only, Domain,
// public suffix, an address, Secure, HttpOnly from a script, SameSite=None without Secure, overwriting
// keeps the place, expiry deletes, the limits) and the Cookie header (order, Secure, HttpOnly, expiry
// on the given clock, SameSite within and across a site)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netpkt::cookie::{canonical_host, default_path, domain_match, parse_date, path_match, site_of, Asks, Jar, Origin, Refused, SameSite, SetCookie, MAX_COOKIE, MAX_COOKIES, MAX_PER_DOMAIN};

use crate::T;

const NOW: i64 = 1_800_000_000;

fn s(x: &str) -> String {
    String::from(x)
}

fn from<'a>(host: &'a str, path: &'a str) -> Origin<'a> {
    Origin { host, path, secure: false, http: true }
}

fn get<'a>(host: &'a str, path: &'a str) -> Asks<'a> {
    Asks { host, path, secure: false, http: true, site: None, top: false, safe: true }
}

pub fn run(t: &mut T) {
    parsing(t);
    dates(t);
    matching(t);
    storing(t);
    reading(t);
    same_site(t);
}

fn parsing(t: &mut T) {
    let c = SetCookie::parse("SID=31d4d96e407aad42; Path=/x; Domain=.Example.COM; Secure; HttpOnly; SameSite=lAX; Max-Age=60; Expires=Wed, 09 Jun 2021 10:18:14 GMT");
    t.check("a whole header", c.is_some());
    let c = c.unwrap_or_default();
    t.eq("name", c.name.as_str(), "SID");
    t.eq("value", c.value.as_str(), "31d4d96e407aad42");
    t.eq("path", c.path.as_deref(), Some("/x"));
    t.eq("domain: lower case, no leading dot", c.domain.as_deref(), Some("example.com"));
    t.check("secure and httponly", c.secure && c.http_only);
    t.eq("samesite", c.same_site, SameSite::Lax);
    t.eq("max-age", c.max_age, Some(60));
    t.eq("expires", c.expires, Some(1_623_233_894));
    // what a user agent takes (5.2)
    t.eq("spaces around them", SetCookie::parse("  a  =  b  ; ").map(|c| (c.name, c.value)), Some((s("a"), s("b"))));
    t.eq("an empty value", SetCookie::parse("a=").map(|c| c.value), Some(s("")));
    t.eq("an = in the value", SetCookie::parse("a=b=c").map(|c| c.value), Some(s("b=c")));
    t.eq("quotes stay", SetCookie::parse("a=\"b\"").map(|c| c.value), Some(s("\"b\"")));
    t.check("no =: ignored", SetCookie::parse("nur-text; Path=/").is_none());
    t.check("no name: ignored", SetCookie::parse("=wert").is_none());
    t.check("nothing: ignored", SetCookie::parse("").is_none());
    let c = SetCookie::parse("a=b; Path=relativ; Max-Age=1x; Domain=; Expires=gestern; SameSite=seltsam; Unbekannt=1").unwrap_or_default();
    t.eq("attributes that cannot be read", (c.path, c.max_age, c.domain, c.expires, c.same_site), (None, None, None, None, SameSite::Unset));
    t.eq("the last one wins", SetCookie::parse("a=b; Max-Age=1; Max-Age=2").and_then(|c| c.max_age), Some(2));
    t.eq("a negative Max-Age", SetCookie::parse("a=b; Max-Age=-5").and_then(|c| c.max_age), Some(-5));
    t.eq("an endless Max-Age", SetCookie::parse("a=b; Max-Age=99999999999999999999999").and_then(|c| c.max_age), Some(i64::MAX));
    t.eq("names of attributes in any case", SetCookie::parse("a=b; secure; HTTPONLY").map(|c| (c.secure, c.http_only)), Some((true, true)));
    t.eq("a Secure with a value", SetCookie::parse("a=b; Secure=nein").map(|c| c.secure), Some(true));
}

fn dates(t: &mut T) {
    for (x, want) in [
        ("Wed, 09 Jun 2021 10:18:14 GMT", 1_623_233_894),
        ("Wed, 09-Jun-21 10:18:14 GMT", 1_623_233_894),
        ("Wednesday, 09-Jun-2021 10:18:14 GMT", 1_623_233_894),
        ("Wed Jun  9 10:18:14 2021", 1_623_233_894),
        ("9 June 2021 10:18:14", 1_623_233_894),
        ("Thu, 01 Jan 1970 00:00:00 GMT", 0),
        ("Tue, 29 Feb 2028 12:00:00 GMT", 1_835_438_400),
        ("Fri, 31 Dec 1999 23:59:59 GMT", 946_684_799),
    ] {
        t.eq(x, parse_date(x), Some(want));
    }
    t.eq("two digits below 70: 20xx", parse_date("01 Jan 69 00:00:00"), parse_date("01 Jan 2069 00:00:00"));
    t.eq("two digits from 70: 19xx", parse_date("01 Jan 70 00:00:00"), Some(0));
    for x in ["", "gestern", "01 Jan 2021", "32 Jan 2021 00:00:00", "31 Feb 2021 00:00:00", "29 Feb 2023 00:00:00", "01 Jan 2021 25:00:00", "01 Jan 2021 00:60:00", "01 Xyz 2021 00:00:00", "01 Jan 1500 00:00:00"] {
        t.eq(&format!("not a date: {:?}", x), parse_date(x), None);
    }
}

fn matching(t: &mut T) {
    t.check("domain: the same", domain_match("example.com", "example.com"));
    t.check("domain: a part of it", domain_match("www.example.com", "example.com"));
    t.check("domain: not only the end of the text", !domain_match("badexample.com", "example.com"));
    t.check("domain: not the other way", !domain_match("example.com", "www.example.com"));
    t.check("domain: an address only itself", domain_match("10.0.0.1", "10.0.0.1") && !domain_match("10.0.0.1", "0.0.1"));
    t.check("path: the same", path_match("/a/b", "/a/b"));
    t.check("path: below", path_match("/a/b/c", "/a/b"));
    t.check("path: a / at its end", path_match("/a/b/c", "/a/"));
    t.check("path: the root", path_match("/x", "/"));
    t.check("path: not only the start of the text", !path_match("/a/bc", "/a/b"));
    t.check("path: not above", !path_match("/a", "/a/b"));
    t.eq("default path: a file", default_path("/a/b/c.html"), s("/a/b"));
    t.eq("default path: at the root", default_path("/c.html"), s("/"));
    t.eq("default path: a / at the end", default_path("/a/b/"), s("/a/b"));
    t.eq("default path: nothing", default_path(""), s("/"));
    t.eq("default path: no /", default_path("x"), s("/"));
    t.eq("canonical host", canonical_host("WEB.Lab."), s("web.lab"));
    t.eq("site: two labels", site_of("www.shop.Web.lab"), s("web.lab"));
    t.eq("site: one label", site_of("web1"), s("web1"));
    t.eq("site: an address", site_of("10.0.0.1"), s("10.0.0.1"));
}

fn storing(t: &mut T) {
    let mut j = Jar::new();
    t.eq("host-only", j.store("a=1", from("www.web.lab", "/shop/korb"), NOW), Ok(()));
    let c = j.cookies()[0].clone();
    t.eq("host-only: domain, path", (c.domain, c.host_only, c.path, c.expires), (s("www.web.lab"), true, s("/shop"), None));
    t.eq("a Domain above the host", j.store("b=2; Domain=web.lab; Path=/", from("www.web.lab", "/"), NOW), Ok(()));
    t.check("a Domain makes it no host-only one", j.cookies().iter().any(|c| c.name == "b" && !c.host_only && c.domain == "web.lab"));
    t.eq("a Domain of another site", j.store("c=3; Domain=other.lab", from("www.web.lab", "/"), NOW), Err(Refused::Domain));
    t.eq("a Domain below the host", j.store("c=3; Domain=x.www.web.lab", from("www.web.lab", "/"), NOW), Err(Refused::Domain));
    t.eq("a public suffix", j.store("c=3; Domain=lab", from("www.web.lab", "/"), NOW), Err(Refused::Domain));
    t.eq("a public suffix as the host's name", j.store("c=3; Domain=web1", from("web1", "/"), NOW), Ok(()));
    t.check("that makes a host-only one", j.cookies().iter().any(|c| c.name == "c" && c.host_only && c.domain == "web1"));
    t.eq("an address: Domain the same", j.store("d=4; Domain=10.0.0.1", from("10.0.0.1", "/"), NOW), Ok(()));
    t.eq("an address: Domain a part of it", j.store("d=4; Domain=0.0.1", from("10.0.0.1", "/"), NOW), Err(Refused::Domain));
    // Secure, HttpOnly, SameSite
    let mut j = Jar::new();
    let sec = Origin { secure: true, ..from("web.lab", "/") };
    let script = Origin { http: false, ..from("web.lab", "/") };
    t.eq("Secure without https", j.store("a=1; Secure", from("web.lab", "/"), NOW), Err(Refused::Secure));
    t.eq("Secure with https", j.store("a=1; Secure", sec, NOW), Ok(()));
    t.eq("a Secure cookie stays from http", j.store("a=2", from("web.lab", "/"), NOW), Err(Refused::Secure));
    t.eq("and from a part of its domain", j.store("a=2", from("www.web.lab", "/"), NOW), Err(Refused::Secure));
    t.eq("another domain does as it likes", j.store("a=2", from("other.lab", "/"), NOW), Ok(()));
    t.eq("HttpOnly from a script", j.store("b=1; HttpOnly", script, NOW), Err(Refused::HttpOnly));
    t.eq("HttpOnly from a header", j.store("b=1; HttpOnly", from("web.lab", "/"), NOW), Ok(()));
    t.eq("a script does not overwrite it", j.store("b=2", script, NOW), Err(Refused::HttpOnly));
    t.eq("SameSite=None without Secure", j.store("c=1; SameSite=None", from("web.lab", "/"), NOW), Err(Refused::SameSiteNone));
    t.eq("SameSite=None with Secure", j.store("c=1; SameSite=None; Secure", sec, NOW), Ok(()));
    t.eq("no name", j.store("=1", from("web.lab", "/"), NOW), Err(Refused::Syntax));
    let big = format!("g={}", "x".repeat(MAX_COOKIE));
    t.eq("too large", j.store(&big, from("web.lab", "/"), NOW), Err(Refused::TooLarge));
    // the same name, domain and path: the new value in the old place
    let mut j = Jar::new();
    for x in ["a=1", "b=2", "a=3"] {
        let _ = j.store(x, from("web.lab", "/"), NOW);
    }
    t.eq("overwriting keeps the place", j.header(&get("web.lab", "/"), NOW), s("a=3; b=2"));
    // another path or domain is another cookie
    let _ = j.store("a=4; Path=/x", from("web.lab", "/"), NOW);
    t.eq("another path: another one", j.cookies().len(), 3);
    // expiry deletes
    let _ = j.store("b=; Max-Age=0", from("web.lab", "/"), NOW);
    t.eq("Max-Age=0 deletes", j.header(&get("web.lab", "/"), NOW), s("a=3"));
    let _ = j.store("a=; Expires=Thu, 01 Jan 1970 00:00:00 GMT", from("web.lab", "/"), NOW);
    t.eq("a date in the past deletes", j.header(&get("web.lab", "/"), NOW), s(""));
    t.eq("Max-Age before Expires", {
        let mut k = Jar::new();
        let _ = k.store("a=1; Max-Age=60; Expires=Thu, 01 Jan 1970 00:00:00 GMT", from("web.lab", "/"), NOW);
        k.cookies().first().and_then(|c| c.expires)
    }, Some(NOW + 60));
    // the limits
    let mut j = Jar::new();
    for i in 0..MAX_PER_DOMAIN + 5 {
        let _ = j.store(&format!("k{}=1", i), from("web.lab", "/"), NOW);
    }
    t.eq("at most so many per domain", j.cookies().len(), MAX_PER_DOMAIN);
    t.check("the oldest went", !j.cookies().iter().any(|c| c.name == "k0") && j.cookies().iter().any(|c| c.name == "k54"));
    let mut j = Jar::new();
    for i in 0..MAX_COOKIES + 5 {
        let _ = j.store(&format!("k{}=1", i), from(&format!("h{}.lab", i), "/"), NOW);
    }
    t.eq("at most so many in all", j.cookies().len(), MAX_COOKIES);
    // the session
    let mut j = Jar::new();
    let _ = j.store("a=1", from("web.lab", "/"), NOW);
    let _ = j.store("b=2; Max-Age=600", from("web.lab", "/"), NOW);
    j.drop_session();
    t.eq("the session ends: what lasts stays", j.header(&get("web.lab", "/"), NOW), s("b=2"));
    t.check("remove", j.remove("b", "web.lab", "/") && j.is_empty());
    t.check("remove: nothing there", !j.remove("b", "web.lab", "/"));
}

fn reading(t: &mut T) {
    let mut j = Jar::new();
    for x in ["kurz=1; Path=/", "lang=2; Path=/a/b", "mitte=3; Path=/a", "zwei=4; Path=/"] {
        let _ = j.store(x, from("web.lab", "/"), NOW);
    }
    t.eq("longer paths first, then the older", j.header(&get("web.lab", "/a/b/c"), NOW), s("lang=2; mitte=3; kurz=1; zwei=4"));
    t.eq("a path only below it", j.header(&get("web.lab", "/ab"), NOW), s("kurz=1; zwei=4"));
    // host-only against Domain
    let mut j = Jar::new();
    let _ = j.store("nur=1", from("web.lab", "/"), NOW);
    let _ = j.store("alle=2; Domain=web.lab", from("web.lab", "/"), NOW);
    t.eq("the host gets both", j.header(&get("WEB.lab", "/"), NOW), s("nur=1; alle=2"));
    t.eq("a part of the domain only the Domain one", j.header(&get("www.web.lab", "/"), NOW), s("alle=2"));
    t.eq("another site none", j.header(&get("other.lab", "/"), NOW), s(""));
    // Secure, HttpOnly
    let mut j = Jar::new();
    let sec = Origin { secure: true, ..from("web.lab", "/") };
    let _ = j.store("sicher=1; Secure", sec, NOW);
    let _ = j.store("nurhttp=2; HttpOnly", from("web.lab", "/"), NOW);
    let _ = j.store("offen=3", from("web.lab", "/"), NOW);
    t.eq("Secure only over https", j.header(&get("web.lab", "/"), NOW), s("nurhttp=2; offen=3"));
    t.eq("over https all", j.header(&Asks { secure: true, ..get("web.lab", "/") }, NOW), s("sicher=1; nurhttp=2; offen=3"));
    t.eq("a script: no HttpOnly", j.script_text("web.lab", "/", true, NOW), s("sicher=1; offen=3"));
    // expiry on the given clock
    let mut j = Jar::new();
    let _ = j.store("a=1; Max-Age=10", from("web.lab", "/"), NOW);
    let _ = j.store("b=2; Expires=Tue, 15 Jan 2030 08:00:00 GMT", from("web.lab", "/"), NOW);
    t.eq("before its time", j.header(&get("web.lab", "/"), NOW + 9), s("a=1; b=2"));
    t.eq("at its time", j.header(&get("web.lab", "/"), NOW + 10), s("b=2"));
    let late = parse_date("Tue, 15 Jan 2030 08:00:00 GMT").unwrap_or(0);
    t.eq("before its date", j.header(&get("web.lab", "/"), late - 1), s("b=2"));
    t.eq("at its date", j.header(&get("web.lab", "/"), late), s(""));
    j.purge(late);
    t.check("purge", j.is_empty());
    let all: Vec<&str> = Vec::new();
    t.eq("an empty jar", Jar::new().matching(&get("x", "/"), NOW).len(), all.len());
}

fn same_site(t: &mut T) {
    let mut j = Jar::new();
    let sec = Origin { secure: true, ..from("web.lab", "/") };
    let _ = j.store("streng=1; SameSite=Strict", from("web.lab", "/"), NOW);
    let _ = j.store("lax=2; SameSite=Lax", from("web.lab", "/"), NOW);
    let _ = j.store("ohne=3", from("web.lab", "/"), NOW);
    let _ = j.store("immer=4; SameSite=None; Secure", sec, NOW);
    let base = Asks { secure: true, ..get("web.lab", "/") };
    t.eq("the user: all", j.header(&base, NOW), s("streng=1; lax=2; ohne=3; immer=4"));
    t.eq("the same site: all", j.header(&Asks { site: Some("web.lab"), ..base }, NOW), s("streng=1; lax=2; ohne=3; immer=4"));
    t.eq("a part of the same site: all", j.header(&Asks { host: "web.lab", site: Some("web.lab"), ..base }, NOW), s("streng=1; lax=2; ohne=3; immer=4"));
    t.eq("another site, a picture: only None", j.header(&Asks { site: Some("other.lab"), ..base }, NOW), s("immer=4"));
    t.eq("another site, a link: no Strict", j.header(&Asks { site: Some("other.lab"), top: true, ..base }, NOW), s("lax=2; ohne=3; immer=4"));
    t.eq("another site, a POST: only None", j.header(&Asks { site: Some("other.lab"), top: true, safe: false, ..base }, NOW), s("immer=4"));
    t.eq("the names", (SameSite::Strict.name(), SameSite::Lax.name(), SameSite::None.name(), SameSite::Unset.name()), ("Strict", "Lax", "None", ""));
}
