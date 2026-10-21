// httpmsgt -- checks of netpkt::http byte for byte: header lines and their rules, heads of
// requests and responses with every refusal (folding, names, versions, the 16 KiB and 100-line
// limits, an 8 KiB target), the incremental parser fed one byte at a time (More until the
// empty line, then Head, Data and End), pipelined messages in one push, chunked bodies with
// extensions and trailers, responses without a body (HEAD, 1xx, 204, 304) and until the close,
// body limits, sticky errors, the encoders (Content-Length, chunks, no header injection),
// keep-alive rules, the Date format and the dissector (fields with their byte ranges, info
// lines, continuations, HTTP on another port, pipelining, chunks, malformed heads)

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};

use netpkt::decode::{self, Proto, P_HTTP};
use netpkt::eth::{self, EthHdr};
use netpkt::http::{self, Event, Framing, Head, Headers, HttpErr, Parser, Version};
use netpkt::ipv4::{self, Ipv4Hdr};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{self, TcpHdr, TcpOpts};
use netpkt::{Ipv4, Mac};

use crate::T;

pub fn run(t: &mut T) {
    headers(t);
    heads(t);
    limits(t);
    byte_by_byte(t);
    pipelining(t);
    chunked(t);
    responses(t);
    bodies(t);
    encoders(t);
    keep_alive(t);
    dissector(t);
}

fn hs(pairs: &[(&str, &str)]) -> Headers {
    let mut h = Headers::new();
    for (k, v) in pairs {
        h.add(k, v);
    }
    h
}

/// every event of a parser until it needs more (or fails)
fn drain(p: &mut Parser, first: Result<Event, HttpErr>) -> Result<Vec<Event>, HttpErr> {
    let mut v = Vec::new();
    let mut e = first?;
    while e != Event::More {
        v.push(e);
        e = p.poll()?;
    }
    Ok(v)
}

/// events of a whole buffer pushed at once
fn events(p: &mut Parser, b: &[u8]) -> Result<Vec<Event>, HttpErr> {
    let first = p.push(b);
    drain(p, first)
}

/// the body of the events, joined
fn body(ev: &[Event]) -> Vec<u8> {
    let mut b = Vec::new();
    for e in ev {
        if let Event::Data(d) = e {
            b.extend_from_slice(d);
        }
    }
    b
}

fn head_of(ev: &[Event]) -> Option<Head> {
    ev.iter().find_map(|e| if let Event::Head(h) = e { Some(h.clone()) } else { None })
}

fn headers(t: &mut T) {
    let mut h = hs(&[("Host", "a"), ("Connection", "Keep-Alive, Upgrade")]);
    t.eq("get ignores case", h.get("HOST"), Some("a"));
    t.check("has token", h.has_token("connection", "upgrade") && h.has_token("Connection", "keep-alive") && !h.has_token("connection", "close"));
    h.set("host", "b");
    t.eq("set replaces", (h.get("Host"), h.len()), (Some("b"), 2));
    h.add("X-A", "1");
    h.add("x-a", "2");
    t.eq("first of two", h.get("x-A"), Some("1"));
    h.remove("X-A");
    t.eq("remove all", (h.get("x-a"), h.len()), (None, 2));
    t.eq("no length", Headers::new().content_length(), Ok(None));
    t.eq("length", hs(&[("Content-Length", " 42 ")]).content_length(), Ok(Some(42)));
    t.eq("length list of one value", hs(&[("Content-Length", "5, 5"), ("content-length", "5")]).content_length(), Ok(Some(5)));
    t.eq("two lengths", hs(&[("Content-Length", "5"), ("Content-Length", "6")]).content_length(), Err(HttpErr::Bad("two different Content-Lengths")));
    t.eq("length no number", hs(&[("Content-Length", "+5")]).content_length(), Err(HttpErr::Bad("Content-Length is no number")));
    t.eq("length empty", hs(&[("Content-Length", "")]).content_length(), Err(HttpErr::Bad("Content-Length is no number")));
    t.check("chunked", hs(&[("Transfer-Encoding", " Chunked ")]).chunked() && !hs(&[("Transfer-Encoding", "gzip, chunked")]).chunked());
    t.eq("statuses", [HttpErr::Bad("x").status(), HttpErr::HeadTooLarge.status(), HttpErr::UriTooLong.status(), HttpErr::Version.status(), HttpErr::BodyTooLarge.status(), HttpErr::Truncated.status()], [400, 431, 414, 505, 413, 400]);
    t.eq("reasons", [http::reason(200), http::reason(404), http::reason(503), http::reason(299)], ["OK", "Not Found", "Service Unavailable", "Unknown"]);
    // RFC 9110 5.6.7: the example date, and the start of the epoch
    t.eq("date rfc", http::http_date(784_111_777), String::from("Sun, 06 Nov 1994 08:49:37 GMT"));
    t.eq("date epoch", http::http_date(0), String::from("Thu, 01 Jan 1970 00:00:00 GMT"));
    t.eq("date simulator", http::http_date(1_767_225_600 + 86_400 * 292 + 3_723), String::from("Tue, 20 Oct 2026 01:02:03 GMT"));
}

fn heads(t: &mut T) {
    let req = b"GET /index.html?x=1 HTTP/1.1\r\nHost: 10.0.0.2\r\nUser-Agent:  probe/1 \r\n\r\nrest";
    let h = http::parse_head(req, true);
    let want = Head {
        method: String::from("GET"),
        target: String::from("/index.html?x=1"),
        version: Version::Http11,
        headers: hs(&[("Host", "10.0.0.2"), ("User-Agent", "probe/1")]),
        len: req.len() - 4,
        ..Head::default()
    };
    t.eq("request head", h, Ok(Some(want.clone())));
    t.eq("path", want.path(), "/index.html");
    t.eq("start line", want.start_line(), String::from("GET /index.html?x=1 HTTP/1.1"));
    t.check("request kind", want.is_request() && want.keep_alive());
    for cut in [0, 5, 28, 29, 47, req.len() - 5] {
        t.eq(&format!("incomplete at {}", cut), http::parse_head(&req[..cut], true), Ok(None));
    }
    // LF alone, empty lines before the start line (four at most)
    let lf = b"\r\n\n\r\n\nPOST / HTTP/1.0\nA: 1\n\n";
    t.eq("lf and empty lines", http::parse_head(lf, true).map(|h| h.map(|h| (h.method, h.version, h.headers.len(), h.len))), Ok(Some((String::from("POST"), Version::Http10, 1, lf.len()))));
    t.eq("five empty lines", http::parse_head(b"\n\n\n\n\nGET / HTTP/1.1\n\n", true), Err(HttpErr::Bad("request line is not METHOD TARGET VERSION")));
    t.eq("folding", http::parse_head(b"GET / HTTP/1.1\r\nA: 1\r\n  2\r\n\r\n", true), Err(HttpErr::Bad("obsolete line folding")));
    t.eq("name with space", http::parse_head(b"GET / HTTP/1.1\r\nBad Name: 1\r\n\r\n", true), Err(HttpErr::Bad("bad header name")));
    t.eq("no colon", http::parse_head(b"GET / HTTP/1.1\r\nNoColon\r\n\r\n", true), Err(HttpErr::Bad("header line without a colon")));
    t.eq("empty name", http::parse_head(b"GET / HTTP/1.1\r\n: x\r\n\r\n", true), Err(HttpErr::Bad("bad header name")));
    t.eq("version", http::parse_head(b"GET / HTTP/2.0\r\n\r\n", true), Err(HttpErr::Version));
    t.eq("lower-case method", http::parse_head(b"get / HTTP/1.1\r\n\r\n", true), Err(HttpErr::Bad("bad method")));
    t.eq("bad target", http::parse_head(b"GET index HTTP/1.1\r\n\r\n", true), Err(HttpErr::Bad("bad request target")));
    t.eq("asterisk", http::parse_head(b"OPTIONS * HTTP/1.1\r\n\r\n", true).map(|h| h.map(|h| h.target)), Ok(Some(String::from("*"))));
    t.eq("two spaces", http::parse_head(b"GET  / HTTP/1.1\r\n\r\n", true), Err(HttpErr::Bad("request line is not METHOD TARGET VERSION")));
    t.eq("not utf-8", http::parse_head(b"GET / HTTP/1.1\r\nA: \xff\r\n\r\n", true), Err(HttpErr::Bad("a line that is not UTF-8")));
    // responses
    let ok = http::parse_head(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi", false);
    t.eq("status line", ok.as_ref().map(|h| h.as_ref().map(|h| (h.status, h.reason.as_str(), h.version, h.len, h.is_request()))), Ok(Some((200, "OK", Version::Http11, 38, false))));
    t.eq("no reason", http::parse_head(b"HTTP/1.0 204\r\n\r\n", false).map(|h| h.map(|h| (h.status, h.reason.clone(), h.version, h.start_line()))), Ok(Some((204, String::new(), Version::Http10, String::from("HTTP/1.0 204")))));
    t.eq("reason with spaces", http::parse_head(b"HTTP/1.1 404 Not Found\r\n\r\n", false).map(|h| h.map(|h| h.reason)), Ok(Some(String::from("Not Found"))));
    t.eq("http/2 status", http::parse_head(b"HTTP/2 200 OK\r\n\r\n", false), Err(HttpErr::Bad("status line without HTTP/1.x")));
    t.eq("two digits", http::parse_head(b"HTTP/1.1 20 OK\r\n\r\n", false), Err(HttpErr::Bad("status is not three digits")));
    t.eq("status 099", http::parse_head(b"HTTP/1.1 099 x\r\n\r\n", false), Err(HttpErr::Bad("status below 100")));
    t.eq("garbage response", http::parse_head(b"garbage here\r\n\r\n", false), Err(HttpErr::Bad("status line without HTTP/1.x")));
    t.eq("start kinds", [http::start_kind(b"GET / HTTP/1.1"), http::start_kind(b"\r\nHTTP/1.1 200"), http::start_kind(b"get /"), http::start_kind(b"\x16\x03\x01"), http::start_kind(b"POST")], [Some(true), Some(false), None, None, None]);
}

fn limits(t: &mut T) {
    // 100 header lines are fine, the 101st is not
    let mut b = String::from("GET / HTTP/1.1\r\n");
    for i in 0..100 {
        b.push_str(&format!("X-{}: {}\r\n", i, i));
    }
    let hundred = format!("{}\r\n", b);
    t.eq("100 lines", http::parse_head(hundred.as_bytes(), true).map(|h| h.map(|h| h.headers.len())), Ok(Some(100)));
    b.push_str("X-100: 100\r\n\r\n");
    t.eq("101 lines", http::parse_head(b.as_bytes(), true), Err(HttpErr::HeadTooLarge));
    // 16 KiB: a header line that does not fit any more, also before its line end came
    let big = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(16 * 1024));
    t.eq("long line", http::parse_head(big.as_bytes(), true), Err(HttpErr::HeadTooLarge));
    t.eq("long line without its end", http::parse_head(&big.as_bytes()[..16 * 1024 + 18], true), Err(HttpErr::HeadTooLarge));
    let fits = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(16 * 1024 - 14 - 5 - 2));
    t.check("just fits", matches!(http::parse_head(fits.as_bytes(), true), Ok(Some(_))));
    let target = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(8 * 1024));
    t.eq("long target", http::parse_head(target.as_bytes(), true), Err(HttpErr::UriTooLong));
    let mut p = Parser::requests();
    t.eq("parser 431 waits for nothing", p.push(&big.as_bytes()[..16 * 1024 + 18]), Err(HttpErr::HeadTooLarge));
    t.eq("errors stick", p.push(b"GET / HTTP/1.1\r\n\r\n"), Err(HttpErr::HeadTooLarge));
}

fn byte_by_byte(t: &mut T) {
    let msg = b"POST /api HTTP/1.1\r\nHost: s\r\nContent-Length: 5\r\n\r\nhello";
    let mut p = Parser::requests();
    let mut seen: Vec<(usize, Event)> = Vec::new();
    for (i, c) in msg.iter().enumerate() {
        let first = p.push(&[*c]);
        match drain(&mut p, first) {
            Ok(ev) => {
                for e in ev {
                    seen.push((i, e));
                }
            }
            Err(e) => {
                t.check(&format!("byte {}: {:?}", i, e), false);
                return;
            }
        }
    }
    let head_at = msg.len() - 6;
    t.eq("head after the empty line", seen.first().map(|s| s.0), Some(head_at));
    t.eq("head", head_of(&seen.iter().map(|s| s.1.clone()).collect::<Vec<_>>()).map(|h| (h.method, h.target, h.headers.get("host").map(String::from), h.len)), Some((String::from("POST"), String::from("/api"), Some(String::from("s")), head_at + 1)));
    let data: Vec<(usize, Vec<u8>)> = seen.iter().filter_map(|s| if let Event::Data(d) = &s.1 { Some((s.0, d.clone())) } else { None }).collect();
    t.eq("one byte per push", data, (0..5).map(|k| (head_at + 1 + k, vec![b"hello"[k]])).collect::<Vec<_>>());
    t.eq("end with the last byte", seen.last().cloned(), Some((msg.len() - 1, Event::End(Headers::new()))));
    t.check("idle again", p.idle() && !p.in_message() && p.messages == 1);
    // a request without a length has no body
    let mut p = Parser::requests();
    t.eq("no body", events(&mut p, b"GET / HTTP/1.1\r\n\r\n").map(|e| e.len()), Ok(2));
    t.eq("framing none", p.framing(), Framing::None);
}

fn pipelining(t: &mut T) {
    let two = b"GET /a HTTP/1.1\r\nHost: s\r\n\r\nPUT /b HTTP/1.1\r\nContent-Length: 3\r\n\r\nxyzGET /c HTTP/1.1\r\n";
    let mut p = Parser::requests();
    let ev = events(&mut p, two);
    let targets: Vec<String> = ev.as_ref().map(|v| v.iter().filter_map(|e| if let Event::Head(h) = e { Some(h.target.clone()) } else { None }).collect()).unwrap_or_default();
    t.eq("two complete, a third begun", targets, vec![String::from("/a"), String::from("/b")]);
    t.eq("events", ev.as_ref().map(|v| v.iter().map(|e| match e { Event::Head(_) => 'H', Event::Data(_) => 'D', Event::End(_) => 'E', Event::More => 'M' }).collect::<String>()), Ok(String::from("HEHDE")));
    t.eq("the third waits", (p.buffered(), p.idle()), (17, false));
    t.eq("the third completes", events(&mut p, b"\r\n").map(|v| head_of(&v).map(|h| h.target)), Ok(Some(String::from("/c"))));
    // responses in order: the second answers HEAD and has no body though it names a length
    let mut c = Parser::responses();
    c.expect("GET");
    c.expect("HEAD");
    c.expect("GET");
    t.eq("outstanding", c.outstanding(), 3);
    let r = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhiHTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nHTTP/1.1 404 Not Found\r\nContent-Length: 1\r\n\r\nx";
    let ev = events(&mut c, r).unwrap_or_default();
    t.eq("three responses", ev.iter().filter(|e| matches!(e, Event::End(_))).count(), 3);
    t.eq("bodies", body(&ev), b"hix".to_vec());
    t.eq("all answered", c.outstanding(), 0);
}

fn chunked(t: &mut T) {
    let r = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nhello \r\n8;ext=1\r\nchunked \r\n0\r\nX-Sum: 3\r\n\r\n";
    let mut c = Parser::responses();
    c.expect("GET");
    let ev = events(&mut c, r).unwrap_or_default();
    t.eq("chunk data", ev.iter().filter_map(|e| if let Event::Data(d) = e { Some(d.clone()) } else { None }).collect::<Vec<_>>(), vec![b"hello ".to_vec(), b"chunked ".to_vec()]);
    t.eq("trailers", ev.last().cloned(), Some(Event::End(hs(&[("X-Sum", "3")]))));
    t.eq("framing", c.body_len(), 14);
    // one byte at a time gives the same body
    let mut c = Parser::responses();
    let mut all = Vec::new();
    let mut ends = 0;
    for b in r.iter() {
        let first = c.push(&[*b]);
        for e in drain(&mut c, first).unwrap_or_default() {
            match e {
                Event::Data(d) => all.extend_from_slice(&d),
                Event::End(tr) => {
                    ends += 1;
                    t.eq("trailers byte by byte", tr, hs(&[("X-Sum", "3")]));
                }
                _ => {}
            }
        }
    }
    t.eq("body byte by byte", (all, ends), (b"hello chunked ".to_vec(), 1));
    let bad = |s: &[u8]| {
        let mut c = Parser::responses();
        events(&mut c, s)
    };
    t.eq("bad size", bad(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"), Err(HttpErr::Bad("bad chunk size")));
    t.eq("no line end after a chunk", bad(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhixx"), Err(HttpErr::Bad("no line end after a chunk")));
    let long = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;{}", "e".repeat(1100));
    t.eq("size line too long", bad(long.as_bytes()), Err(HttpErr::Bad("chunk-size line too long")));
    t.eq("gzip", bad(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n"), Err(HttpErr::Bad("Transfer-Encoding other than chunked")));
    // a chunked request
    let mut p = Parser::requests();
    let ev = events(&mut p, b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n").unwrap_or_default();
    t.eq("chunked request", (body(&ev), p.messages), (b"abc".to_vec(), 1));
    t.eq("te with cl", events(&mut Parser::requests(), b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 3\r\n\r\n"), Err(HttpErr::Bad("Transfer-Encoding and Content-Length")));
}

fn responses(t: &mut T) {
    // 1xx comes before the answer and answers nothing
    let mut c = Parser::responses();
    c.expect("POST");
    let ev = events(&mut c, b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nok").unwrap_or_default();
    let st: Vec<u16> = ev.iter().filter_map(|e| if let Event::Head(h) = e { Some(h.status) } else { None }).collect();
    t.eq("continue then created", (st, body(&ev), c.outstanding()), (vec![100, 201], b"ok".to_vec(), 0));
    for (status, name) in [(204, "204"), (304, "304")] {
        let mut c = Parser::responses();
        c.expect("GET");
        let ev = events(&mut c, format!("HTTP/1.1 {} X\r\nContent-Length: 5\r\n\r\n", status).as_bytes()).unwrap_or_default();
        t.eq(&format!("{} has no body", name), ev.len(), 2);
    }
    // without a length until the close
    let mut c = Parser::responses();
    c.expect("GET");
    let ev = events(&mut c, b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\nuntil ").unwrap_or_default();
    t.eq("until close so far", (body(&ev), c.framing(), c.in_message()), (b"until ".to_vec(), Framing::UntilClose, true));
    let more = events(&mut c, b"close\n").unwrap_or_default();
    t.eq("more data", body(&more), b"close\n".to_vec());
    t.eq("the close ends it", c.finish(), Ok(Event::End(Headers::new())));
    t.check("idle after the close", c.idle());
    // a close inside a body, between messages, before a head is complete
    let mut c = Parser::responses();
    let _ = events(&mut c, b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort");
    t.eq("truncated", c.finish(), Err(HttpErr::Truncated));
    t.eq("clean end", Parser::responses().finish(), Ok(Event::More));
    let mut c = Parser::responses();
    let _ = c.push(b"HTTP/1.1 200");
    t.eq("half a head", c.finish(), Err(HttpErr::Truncated));
}

fn bodies(t: &mut T) {
    let mut p = Parser::requests();
    p.max_body = 10;
    t.eq("length over the limit", p.push(b"POST / HTTP/1.1\r\nContent-Length: 11\r\n\r\n"), Err(HttpErr::BodyTooLarge));
    let mut p = Parser::requests();
    p.max_body = 10;
    t.eq("chunks over the limit", events(&mut p, b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nabcdef\r\n5\r\n"), Err(HttpErr::BodyTooLarge));
    let mut c = Parser::responses();
    c.max_body = 4;
    t.eq("until close over the limit", events(&mut c, b"HTTP/1.1 200 OK\r\n\r\nabcde"), Err(HttpErr::BodyTooLarge));
    let mut p = Parser::requests();
    p.max_body = 10;
    t.eq("exactly the limit", events(&mut p, b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\n0123456789").map(|e| body(&e).len()), Ok(10));
    t.eq("framings", [
        http::request_framing(&hs(&[("Content-Length", "0")])),
        http::request_framing(&hs(&[("Content-Length", "7")])),
        http::response_framing(200, &Headers::new(), "GET"),
        http::response_framing(200, &hs(&[("Content-Length", "7")]), "HEAD"),
        http::response_framing(200, &Headers::new(), "CONNECT"),
        http::response_framing(101, &Headers::new(), "GET"),
    ], [Ok(Framing::None), Ok(Framing::Length(7)), Ok(Framing::UntilClose), Ok(Framing::None), Ok(Framing::None), Ok(Framing::None)]);
}

fn encoders(t: &mut T) {
    t.eq("get", http::encode_request("GET", "/", Version::Http11, &hs(&[("Host", "10.0.0.2")]), &[]), b"GET / HTTP/1.1\r\nHost: 10.0.0.2\r\n\r\n".to_vec());
    t.eq("post without a body", http::encode_request("POST", "/x", Version::Http10, &Headers::new(), &[]), b"POST /x HTTP/1.0\r\nContent-Length: 0\r\n\r\n".to_vec());
    t.eq("put with a body", http::encode_request("PUT", "/k", Version::Http11, &hs(&[("Content-Length", "99")]), b"v"), b"PUT /k HTTP/1.1\r\nContent-Length: 1\r\n\r\nv".to_vec());
    t.eq("injection left out", http::encode_request("GET", "/", Version::Http11, &hs(&[("A", "1\r\nEvil: 1"), ("B:C", "2"), ("C", "3")]), &[]), b"GET / HTTP/1.1\r\nC: 3\r\n\r\n".to_vec());
    t.eq("chunked request", http::encode_request("POST", "/", Version::Http11, &hs(&[("Transfer-Encoding", "chunked")]), b"abc"), b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n".to_vec());
    t.eq("response", http::encode_response(Version::Http11, 200, &hs(&[("Content-Type", "text/plain")]), b"hi", false), b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nhi".to_vec());
    t.eq("head response", http::encode_response(Version::Http11, 200, &Headers::new(), b"hello", true), b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n".to_vec());
    t.eq("204", http::encode_response(Version::Http11, 204, &Headers::new(), b"x", false), b"HTTP/1.1 204 No Content\r\n\r\n".to_vec());
    t.eq("100", http::encode_response(Version::Http11, 100, &Headers::new(), &[], false), b"HTTP/1.1 100 Continue\r\n\r\n".to_vec());
    t.eq("chunked response", http::encode_response(Version::Http11, 200, &hs(&[("Transfer-Encoding", "chunked")]), b"hello", false), b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec());
    t.eq("chunk", http::encode_chunk(&[7; 26]).len(), 2 + 2 + 26 + 2);
    t.eq("chunk head", &http::encode_chunk(&[7; 26])[..4], &b"1a\r\n"[..]);
    t.eq("empty chunk", http::encode_chunk(&[]), Vec::new());
    t.eq("chunked in fours", http::encode_chunked(b"abcdefghij", 4), b"4\r\nabcd\r\n4\r\nefgh\r\n2\r\nij\r\n0\r\n\r\n".to_vec());
    t.eq("last chunk", http::encode_last_chunk(&Headers::new()), b"0\r\n\r\n".to_vec());
    t.eq("last chunk with trailers", http::encode_last_chunk(&hs(&[("X-Sum", "3")])), b"0\r\nX-Sum: 3\r\n\r\n".to_vec());
    let head = Head { status: 503, reason: String::from("Busy"), version: Version::Http10, headers: hs(&[("Retry-After", "1")]), ..Head::default() };
    t.eq("head as given", http::encode_head(&head), b"HTTP/1.0 503 Busy\r\nRetry-After: 1\r\n\r\n".to_vec());
    // round trips
    let req = http::encode_request("POST", "/r?q=1", Version::Http11, &hs(&[("Host", "h"), ("X", "y")]), b"payload");
    let ev = events(&mut Parser::requests(), &req).unwrap_or_default();
    t.eq("request round trip", (head_of(&ev).map(|h| (h.method, h.target, h.headers)), body(&ev)), (Some((String::from("POST"), String::from("/r?q=1"), hs(&[("Host", "h"), ("X", "y"), ("Content-Length", "7")]))), b"payload".to_vec()));
    let resp = http::encode_response(Version::Http11, 404, &hs(&[("Content-Type", "text/plain")]), b"nope", false);
    let mut c = Parser::responses();
    c.expect("GET");
    let ev = events(&mut c, &resp).unwrap_or_default();
    t.eq("response round trip", (head_of(&ev).map(|h| (h.status, h.reason)), body(&ev)), (Some((404, String::from("Not Found"))), b"nope".to_vec()));
}

fn keep_alive(t: &mut T) {
    let ka = |v, pairs: &[(&str, &str)]| http::keep_alive(v, &hs(pairs));
    t.eq(
        "keep-alive rules",
        [ka(Version::Http11, &[]), ka(Version::Http11, &[("Connection", "close")]), ka(Version::Http11, &[("Connection", "Upgrade, Close")]), ka(Version::Http10, &[]), ka(Version::Http10, &[("Connection", "keep-alive")])],
        [true, false, false, false, true],
    );
}

// ---------------------------------------------------------------- the dissector

const MAC_A: Mac = Mac([0x02, 0x4A, 0, 0, 1, 1]);
const MAC_B: Mac = Mac([0x02, 0x4A, 0, 0, 2, 1]);
const IP_A: Ipv4 = Ipv4([10, 0, 0, 1]);
const IP_B: Ipv4 = Ipv4([10, 0, 0, 2]);

/// an Ethernet frame with a TCP segment carrying `data`
fn tcp_frame(sport: u16, dport: u16, data: &[u8]) -> Vec<u8> {
    let mut p = Pkt::new(HEADROOM, data);
    TcpHdr { src_port: sport, dst_port: dport, seq: 1, ack: 1, flags: tcp::ACK | tcp::PSH, window: 64240, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut p, IP_A, IP_B);
    Ipv4Hdr::new(IP_A, IP_B, ipv4::PROTO_TCP, 64).push_onto(&mut p);
    EthHdr { dst: MAC_B, src: MAC_A, ethertype: eth::ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    eth::pad(&mut f);
    f
}

/// the HTTP layer's fields: (name, value, offset in the payload, length)
fn fields(f: &[u8]) -> Vec<(String, String, u32, u32)> {
    let l = decode::layers(f);
    let Some(h) = l.iter().find(|l| l.proto == Proto::Http) else {
        return Vec::new();
    };
    h.fields.iter().map(|x| (String::from(x.name), x.value.clone(), x.off - 54, x.len)).collect()
}

fn fl(name: &str, value: &str, off: u32, len: u32) -> (String, String, u32, u32) {
    (String::from(name), String::from(value), off, len)
}

fn dissector(t: &mut T) {
    let req = tcp_frame(40000, 80, b"GET /index.html HTTP/1.1\r\nHost: 10.0.0.2\r\nX-Probe: 1\r\n\r\n");
    t.eq("request info", decode::info(&req), (Proto::Http, String::from("GET /index.html HTTP/1.1")));
    t.check("mask", decode::proto_mask(&req) & P_HTTP != 0);
    t.eq(
        "request fields",
        fields(&req),
        vec![fl("Request Method", "GET", 0, 3), fl("Request URI", "/index.html", 4, 11), fl("Request Version", "HTTP/1.1", 16, 8), fl("Host", "10.0.0.2", 26, 16), fl("Header", "X-Probe: 1", 42, 12)],
    );
    t.check("request clean", decode::layers(&req).iter().all(|l| l.problems.is_empty()));
    let resp = tcp_frame(80, 40000, b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 5\r\n\r\nhello");
    t.eq("response info", decode::info(&resp).1, String::from("HTTP/1.1 200 OK  (text/html)"));
    t.eq(
        "response fields",
        fields(&resp),
        vec![
            fl("Response Version", "HTTP/1.1", 0, 8),
            fl("Status Code", "200", 9, 3),
            fl("Response Phrase", "OK", 13, 2),
            fl("Content-Type", "text/html; charset=utf-8", 17, 40),
            fl("Content-Length", "5", 57, 19),
            fl("File Data", "5 bytes", 78, 5),
        ],
    );
    let cont = tcp_frame(80, 40000, b"more body bytes");
    t.eq("continuation", decode::info(&cont).1, String::from("Continuation"));
    t.eq("continuation field", fields(&cont), vec![fl("Continuation", "15 bytes", 0, 15)]);
    let other = tcp_frame(40001, 8081, b"HEAD /x HTTP/1.0\r\n\r\n");
    t.eq("another port", decode::info(&other), (Proto::Http, String::from("HEAD /x HTTP/1.0")));
    t.eq("not http on another port", decode::info(&tcp_frame(40001, 8081, b"hello")).0, Proto::Data);
    let two = tcp_frame(40000, 80, b"GET /a HTTP/1.1\r\n\r\nGET /b HTTP/1.1\r\n\r\n");
    t.eq("pipelined info", decode::info(&two).1, String::from("GET /a HTTP/1.1 , GET /b HTTP/1.1"));
    let ch = tcp_frame(80, 40000, b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nhello \r\n0\r\n\r\n");
    t.eq(
        "chunk fields",
        fields(&ch).into_iter().skip(4).collect::<Vec<_>>(),
        vec![fl("Data chunk", "6 octets", 47, 11), fl("End of chunked encoding", "0", 58, 5)],
    );
    let part = tcp_frame(80, 40000, b"HTTP/1.1 200 OK\r\nServer: x\r\nContent-Le");
    t.eq("head goes on", (decode::info(&part).1, fields(&part).len()), (String::from("HTTP/1.1 200 OK"), 4));
    let bad = tcp_frame(40000, 80, b"GET / HTTP/1.1\r\n Folded: x\r\n\r\n");
    let probs: Vec<String> = decode::layers(&bad).into_iter().filter(|l| l.proto == Proto::Http).flat_map(|l| l.problems).collect();
    t.eq("malformed", probs, vec![String::from("malformed HTTP request: obsolete line folding")]);
}
