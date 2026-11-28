// httpclientt -- netio's HTTP client against what servers do: URLs and references (RFC 3986 5.4),
// the answer path (status lines, interim answers, framing as RFC 9112 6.3 says it), bodies in
// Content-Encoding (gzip, br, zstd, deflate: decoded in pieces, bombs, damage), redirects (methods,
// credentials across origins, https to http), cookies with the public suffix list, the
// connection pool and Session (no desync), the whole-request deadline, HTTP proxies (absolute
// form, CONNECT tunnels, credentials, NO_PROXY, the environment)

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

use netio::client::{Pool, Session};
use netio::http::{Headers, Reader, Request, Response};
use netio::proxy::{Endpoint, Proxy};
use netio::{Client, HttpError, Part, Server, ServerConfig, Url};
use netpkt::cookie::Jar;
use plat::net::{IpAddr, NetErrorKind, SocketAddr, TcpListener, TcpStream};
use plat::sync::Mutex;
use rt::sync::SpinLock;

use crate::T;

fn u(s: &str) -> Url {
    Url::parse(s).unwrap_or_else(|| panic!("a test URL that does not parse: {s}"))
}

// ------------------------------------------------------------------ URLs

fn urls(t: &mut T) {
    let host = |s: &str| Url::parse(s).map(|u| u.host);
    t.check("url: inet_aton forms refused (0x7f.1, 2130706433, 1.2.3, 0177.0.0.1)",
        ["http://0x7f.1/", "http://2130706433/", "http://1.2.3/", "http://0177.0.0.1/", "http://127.1/"].iter().all(|s| Url::parse(s).is_none()));
    t.eq("url: a dotted IPv4 address", host("http://10.0.0.1:8080/"), Some(String::from("10.0.0.1")));
    t.eq("url: an IPv6 literal", host("http://[2001:DB8::1]/"), Some(String::from("2001:db8::1")));
    t.check("url: IPv6 without brackets refused", Url::parse("http://::1/").is_none() && Url::parse("http://[::1/").is_none() && Url::parse("http://[nothost]/").is_none());
    t.check("url: odd hosts refused (%, space, !, IDN in Unicode, a leading dot, empty)",
        ["http://a%62.example/", "http://a!b/", "http://bücher.example/", "http://.example/", "http:///x", "http://exa\tmple/"].iter().all(|s| Url::parse(s).is_none()));
    t.eq("url: an A-label host", host("http://xn--bcher-kva.example/"), Some(String::from("xn--bcher-kva.example")));
    t.eq("url: \"host:\" is the default port", Url::parse("http://h:/x").map(|u| u.port), Some(80));
    t.check("url: port 0 and a port beyond 65535 refused", Url::parse("http://h:0/").is_none() && Url::parse("http://h:65536/").is_none() && Url::parse("http://h:+80/").is_none());
    t.eq("url: bytes beyond ASCII in the target encoded", Url::parse("http://h/ä?q=ü\"").map(|u| u.target), Some(String::from("/%C3%A4?q=%C3%BC%22")));
    t.eq("url: escapes stay", Url::parse("http://h/a%20b").map(|u| u.target), Some(String::from("/a%20b")));
    t.eq("url: display", Url::parse("https://H:443/x?y#frag").map(|u| format!("{}", u)), Some(String::from("https://h/x?y")));
    t.check("url: same origin", u("http://a/x").same_origin(&u("http://a:80/y")) && !u("http://a/").same_origin(&u("https://a/")) && !u("http://a/").same_origin(&u("http://a:81/")));
    t.eq("url: authority", u("http://[::1]/").authority(), String::from("[::1]:80"));

    // RFC 3986 5.4.1 and 5.4.2 against http://a/b/c/d;p?q (the fragments dropped, "g:h" is
    // another scheme and "http:g" is strict: neither is an http URL)
    let base = u("http://a/b/c/d;p?q");
    let cases: &[(&str, &str)] = &[
        ("g", "http://a/b/c/g"), ("./g", "http://a/b/c/g"), ("g/", "http://a/b/c/g/"), ("/g", "http://a/g"),
        ("//g", "http://g/"), ("?y", "http://a/b/c/d;p?y"), ("g?y", "http://a/b/c/g?y"), ("#s", "http://a/b/c/d;p?q"),
        ("g#s", "http://a/b/c/g"), ("g?y#s", "http://a/b/c/g?y"), (";x", "http://a/b/c/;x"), ("g;x", "http://a/b/c/g;x"),
        ("g;x?y#s", "http://a/b/c/g;x?y"), ("", "http://a/b/c/d;p?q"), (".", "http://a/b/c/"), ("./", "http://a/b/c/"),
        ("..", "http://a/b/"), ("../", "http://a/b/"), ("../g", "http://a/b/g"), ("../..", "http://a/"),
        ("../../", "http://a/"), ("../../g", "http://a/g"),
        ("../../../g", "http://a/g"), ("../../../../g", "http://a/g"), ("/./g", "http://a/g"), ("/../g", "http://a/g"),
        ("g.", "http://a/b/c/g."), (".g", "http://a/b/c/.g"), ("g..", "http://a/b/c/g.."), ("..g", "http://a/b/c/..g"),
        ("./../g", "http://a/b/g"), ("./g/.", "http://a/b/c/g/"), ("g/./h", "http://a/b/c/g/h"), ("g/../h", "http://a/b/c/h"),
        ("g;x=1/./y", "http://a/b/c/g;x=1/y"), ("g;x=1/../y", "http://a/b/c/y"), ("g?y/./x", "http://a/b/c/g?y/./x"),
        ("g?y/../x", "http://a/b/c/g?y/../x"), ("g#s/./x", "http://a/b/c/g"), ("g#s/../x", "http://a/b/c/g"),
        ("https://other:8443/x/../y", "https://other:8443/y"), ("HTTP://A/B", "http://a/B"),
    ];
    let mut bad = Vec::new();
    for (r, want) in cases {
        let got = base.join(r).map(|x| format!("{}", x));
        if got.as_deref() != Some(*want) {
            bad.push(format!("{r:?} -> {got:?}"));
        }
    }
    t.check(&format!("url join: RFC 3986 5.4 ({} cases){}", cases.len(), if bad.is_empty() { String::new() } else { format!(": {}", bad.join(", ")) }), bad.is_empty());
    t.check("url join: other schemes, userinfo, spaces, \"http:g\" refused",
        ["g:h", "ftp://x/", "//evil@x/", " /x", "/a b", "http:g", "\\\\evil/x", "javascript:alert(1)"].iter().all(|r| base.join(r).is_none()));
}

// ------------------------------------------------------------------ proxies, parsed

fn proxies(t: &mut T) {
    let e = Endpoint::parse("http://user:p%40ss@Proxy.LAN:3128/");
    t.check("proxy: user:password@host:port", e.as_ref().is_some_and(|e| e.host == "proxy.lan" && e.port == 3128 && e.authorization() == Some("Basic dXNlcjpwQHNz")));
    t.check("proxy: host:port without a scheme, port 80 by default",
        Endpoint::parse("proxy:8080").is_some_and(|e| e.port == 8080) && Endpoint::parse("http://p").is_some_and(|e| e.port == 80 && e.authorization().is_none()));
    t.check("proxy: https, socks and paths refused",
        ["https://p:443", "socks5://p:1080", "http://p/path", "http://p?x", ""].iter().all(|s| Endpoint::parse(s).is_none()));
    t.check("proxy: credentials stay out of Debug", !format!("{:?}", e).contains("dXNl"));
    let vars = |kv: &'static [(&'static str, &'static str)]| move |n: &str| kv.iter().find(|(k, _)| *k == n).map(|(_, v)| String::from(*v));
    t.check("proxy env: HTTP_PROXY in upper case is left alone (httpoxy)", Proxy::from_vars(&vars(&[("HTTP_PROXY", "http://evil:1")])).is_none());
    t.check("proxy env: none set", Proxy::from_vars(&vars(&[])).is_none() && Proxy::from_vars(&vars(&[("http_proxy", "  ")])).is_none());
    let p = Proxy::from_vars(&vars(&[("http_proxy", "p:1"), ("HTTPS_PROXY", "http://s:2"), ("NO_PROXY", "corp.example, .lan,10.0.0.0/8, [::1]:8080, *.dev.example, 192.168.1.7, bad/x")]));
    let via = |s: &str| p.as_ref().and_then(|p| p.for_url(&u(s))).map(|e| e.key());
    t.eq("proxy env: http_proxy for http", via("http://example.org/"), Some(String::from("p:1")));
    t.eq("proxy env: HTTPS_PROXY for https", via("https://example.org/"), Some(String::from("s:2")));
    t.check("proxy env: NO_PROXY names and the names below them",
        via("http://corp.example/").is_none() && via("http://a.b.corp.example/").is_none() && via("http://x.lan/").is_none() && via("http://y.dev.example/").is_none());
    t.check("proxy env: NO_PROXY does not take a mere ending", via("http://xcorp.example/").is_some() && via("http://notlan/").is_some());
    t.check("proxy env: a leading dot also takes the name itself (as curl)", via("http://lan/").is_none());
    t.check("proxy env: NO_PROXY addresses and ranges", via("http://10.1.2.3/").is_none() && via("http://11.0.0.1/").is_some() && via("http://192.168.1.7/").is_none() && via("http://192.168.1.8/").is_some());
    t.check("proxy: loopback and localhost go direct", via("http://127.0.0.1:5000/").is_none() && via("http://[::1]/").is_none() && via("http://localhost/").is_none() && via("http://app.localhost/").is_none());
    let all = Proxy::from_vars(&vars(&[("all_proxy", "http://a:3")]));
    t.check("proxy env: all_proxy for both", all.as_ref().is_some_and(|p| p.for_url(&u("http://x/")).is_some() && p.for_url(&u("https://x/")).is_some()));
    let only = Proxy::from_vars(&vars(&[("http_proxy", "p:1")]));
    t.check("proxy env: http_proxy alone leaves https direct", only.as_ref().is_some_and(|p| p.for_url(&u("https://x/")).is_none()));
    t.check("proxy: no_proxy(\"*\")", Proxy::new("http://p:1").map(|p| p.no_proxy("*")).is_some_and(|p| p.for_url(&u("http://x/")).is_none()));
}

// ------------------------------------------------------------------ answers from a script

/// fixed bytes to read, the end orderly; what the client writes is kept
struct Scripted {
    data: SpinLock<Vec<u8>>,
    sent: SpinLock<Vec<u8>>,
}

impl Scripted {
    fn new(b: &[u8]) -> Scripted {
        Scripted { data: SpinLock::new(b.to_vec()), sent: SpinLock::new(Vec::new()) }
    }
    fn sent(&self) -> String {
        String::from_utf8_lossy(&self.sent.lock()).into_owned()
    }
}

impl netio::Stream for Scripted {
    fn read(&self, buf: &mut [u8]) -> plat::net::NetResult<usize> {
        let mut d = self.data.lock();
        // in small pieces, as a network gives them
        let n = buf.len().min(d.len()).min(1000);
        buf[..n].copy_from_slice(&d[..n]);
        d.drain(..n);
        Ok(n)
    }
    fn write_all(&self, b: &[u8]) -> plat::net::NetResult<()> {
        self.sent.lock().extend_from_slice(b);
        Ok(())
    }
    fn set_timeouts(&self, _: Option<u32>, _: Option<u32>) {}
}

fn head_then(head: &str, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::from(head.as_bytes());
    v.extend_from_slice(body);
    v
}

fn answers(t: &mut T) {
    let url = u("http://example.org/x");
    let on = |c: &Client, b: &[u8]| c.request_on(&Scripted::new(b), "GET", &url, &Headers::new(), &[]);
    let c = Client::new();
    let ok = |r: &Result<Response, HttpError>, body: &[u8]| matches!(r, Ok(x) if x.status == 200 && x.body_bytes() == body);
    t.check("answer: plain", ok(&on(&c, b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"), b"ok"));
    t.check("answer: a status line without a reason", ok(&on(&c, b"HTTP/1.1 200\r\nContent-Length: 2\r\n\r\nok"), b"ok"));
    for (name, line) in [("+200", "HTTP/1.1 +200 OK"), ("two digits", "HTTP/1.1 20 OK"), ("four digits", "HTTP/1.1 2000 OK"), ("600", "HTTP/1.1 600 X"),
                         ("099", "HTTP/1.1 099 X"), ("HTTP/2", "HTTP/2 200 OK"), ("no space", "HTTP/1.1 200OK"), ("lower case", "http/1.1 200 OK")] {
        let r = on(&c, format!("{line}\r\nContent-Length: 0\r\n\r\n").as_bytes());
        t.check(&format!("answer: status line refused ({name})"), r.err() == Some(HttpError::Bad));
    }
    t.check("answer: interim answers skipped", ok(&on(&c, b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 103 Early Hints\r\nLink: </a>\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx"), b"x"));
    let flood: Vec<u8> = b"HTTP/1.1 100 Continue\r\n\r\n".repeat(17).into_iter().chain(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".iter().copied()).collect();
    t.check("answer: more than 16 interim answers refused", on(&c, &flood).err() == Some(HttpError::Bad));
    t.check("answer: 101 without an upgrade asked for", on(&c, b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: x\r\n\r\n").err() == Some(HttpError::Bad));
    for (name, te) in [("two lines", "Transfer-Encoding: chunked\r\nTransfer-Encoding: identity"), ("gzip, chunked", "Transfer-Encoding: gzip, chunked"),
                       ("chunked twice", "Transfer-Encoding: chunked, chunked"), ("identity", "Transfer-Encoding: identity")] {
        let r = on(&c, format!("HTTP/1.1 200 OK\r\n{te}\r\n\r\n2\r\nok\r\n0\r\n\r\n").as_bytes());
        t.check(&format!("answer: Transfer-Encoding refused ({name})"), r.err() == Some(HttpError::Bad));
    }
    t.check("answer: Content-Length that disagree", on(&c, b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\nokk").err() == Some(HttpError::Bad));
    t.check("answer: no length runs to the end", ok(&on(&c, b"HTTP/1.1 200 OK\r\n\r\nall of it"), b"all of it"));
    t.check("answer: a head over 16 KiB", on(&c, head_then(&format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "v".repeat(17000)), b"").as_slice()).err() == Some(HttpError::TooLarge));

    // what goes out
    let s = Scripted::new(b"HTTP/1.1 204 No Content\r\n\r\n");
    let _ = c.request_on(&s, "GET", &url, &Headers::new(), &[]);
    let sent = s.sent();
    t.check("request: GET offers gzip and br, asks to close", sent.starts_with("GET /x HTTP/1.1\r\n") && sent.contains("Accept-Encoding: gzip, br\r\n") && sent.contains("Connection: close\r\n") && sent.contains("Host: example.org\r\n"));
    let s = Scripted::new(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 0\r\n\r\n");
    let mut h = Headers::new();
    h.set("Range", "bytes=5-");
    let _ = c.request_on(&s, "GET", &url, &h, &[]);
    t.check("request: a Range request offers no coding", !s.sent().contains("Accept-Encoding"));
    let s = Scripted::new(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n");
    let _ = c.request_on(&s, "HEAD", &url, &Headers::new(), &[]);
    t.check("request: HEAD offers no coding, reads no body", !s.sent().contains("Accept-Encoding"));
    let mut nd = Client::new();
    nd.decode = false;
    let s = Scripted::new(b"HTTP/1.1 204 No Content\r\n\r\n");
    let _ = nd.request_on(&s, "GET", &url, &Headers::new(), &[]);
    t.check("request: decode off offers no coding", !s.sent().contains("Accept-Encoding"));
    let mut bad = Headers::new();
    bad.add("X-Evil", "a\r\nInjected: 1");
    bad.add("Bad Name", "v");
    bad.add("X-Nul", "a\0b");
    bad.add("X-Tab", "a\tb");
    let s = Scripted::new(b"HTTP/1.1 204 No Content\r\n\r\n");
    let _ = c.request_on(&s, "GET", &url, &bad, &[]);
    let sent = s.sent();
    t.check("request: headers with CR LF, NUL or a name that is no token are dropped", !sent.contains("Injected") && !sent.contains("Bad Name") && !sent.contains("X-Nul") && sent.contains("X-Tab: a\tb\r\n"));
}

// ------------------------------------------------------------------ Content-Encoding

fn encoded(t: &mut T) {
    let url = u("http://example.org/x");
    let c = Client::new();
    let data: Vec<u8> = (0..200_000u32).map(|i| b"JFLib streams in pieces. "[(i % 25) as usize] ^ ((i / 997) as u8 & 1)).collect();
    let with = |coding: &str, body: &[u8]| head_then(&format!("HTTP/1.1 200 OK\r\nContent-Encoding: {coding}\r\nContent-Length: {}\r\n\r\n", body.len()), body);
    let get = |c: &Client, b: &[u8]| c.request_on(&Scripted::new(b), "GET", &url, &Headers::new(), &[]);
    let decoded = |r: &Result<Response, HttpError>, want: &[u8]| matches!(r, Ok(x) if x.body_bytes() == want && x.headers.get("content-encoding").is_none() && x.headers.get("content-length").is_none());
    for (name, coding, body) in [
        ("gzip", "gzip", compress::gzip::compress(&data, 6)),
        ("x-gzip", "x-gzip", compress::gzip::compress(&data, 1)),
        ("br", "br", compress::brotli::compress(&data, 5)),
        ("zstd", "zstd", compress::zstd::compress(&data, 3)),
        ("deflate (zlib)", "deflate", compress::zlib::compress(&data, 6)),
        ("upper case", "GZIP", compress::gzip::compress(&data, 6)),
    ] {
        t.check(&format!("decode: {name}"), decoded(&get(&c, &with(coding, &body)), &data));
    }
    let gz = compress::gzip::compress(&data, 6);
    let chunked: Vec<u8> = {
        let mut v = Vec::from(&b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\n\r\n"[..]);
        for piece in gz.chunks(777) {
            v.extend_from_slice(format!("{:x}\r\n", piece.len()).as_bytes());
            v.extend_from_slice(piece);
            v.extend_from_slice(b"\r\n");
        }
        v.extend_from_slice(b"0\r\n\r\n");
        v
    };
    t.check("decode: gzip in chunks", decoded(&get(&c, &chunked), &data));
    let mut two = compress::gzip::compress(b"first ", 6);
    two.extend_from_slice(&compress::gzip::compress(b"second", 6));
    t.check("decode: two gzip members", decoded(&get(&c, &with("gzip", &two)), b"first second"));
    t.check("decode: an empty body in gzip is empty", matches!(get(&c, &with("gzip", b"")), Ok(ref x) if x.body_bytes().is_empty()));
    let mut cut = gz.clone();
    cut.truncate(gz.len() / 2);
    t.eq("decode: a gzip body cut short", get(&c, &with("gzip", &cut)).err(), Some(HttpError::Encoding));
    let mut broken = gz.clone();
    broken[gz.len() / 2] ^= 0x55;
    t.check("decode: a damaged gzip body", matches!(get(&c, &with("gzip", &broken)).err(), Some(HttpError::Encoding)));
    let mut tail = compress::brotli::compress(&data, 5);
    tail.extend_from_slice(b"junk");
    t.eq("decode: bytes after a br stream", get(&c, &with("br", &tail)).err(), Some(HttpError::Encoding));
    let mut zl = compress::zlib::compress(&data, 6);
    zl.extend_from_slice(b"x");
    t.eq("decode: bytes after a zlib stream", get(&c, &with("deflate", &zl)).err(), Some(HttpError::Encoding));
    // a bomb: 64 MiB of zeros in some 64 KiB
    let zeros = vec![0u8; 64 << 20];
    let bomb = compress::gzip::compress(&zeros, 6);
    let mut small = Client::new();
    small.max_body = 1 << 20;
    t.check(&format!("decode: a gzip bomb ({} KB to 64 MiB) stops at max_body", bomb.len() / 1000), get(&small, &with("gzip", &bomb)).err() == Some(HttpError::TooLarge));
    let zbomb = compress::zstd::compress(&zeros, 3);
    let r = get(&small, &with("zstd", &zbomb)).err();
    t.check(&format!("decode: a zstd bomb stops (at max_body, or at the window: {:?})", r), r == Some(HttpError::TooLarge) || r == Some(HttpError::Encoding));
    drop(zeros);
    // zstd's window (RFC 9659): a frame of "abc" in one raw block, its Window_Descriptor 8 MiB
    // (exponent 13) or 16 MiB (14): the larger one is refused, not given the memory
    let frame = |wd: u8| head_then("", &[0x28, 0xb5, 0x2f, 0xfd, 0x00, wd, 25, 0, 0, b'a', b'b', b'c']);
    t.check("decode: zstd with an 8 MiB window", decoded(&get(&c, &with("zstd", &frame(13 << 3))), b"abc"));
    t.eq("decode: zstd with a 16 MiB window refused", get(&c, &with("zstd", &frame(14 << 3))).err(), Some(HttpError::Encoding));
    // what comes back as it is
    let stacked = with("gzip, br", b"raw");
    t.check("decode: stacked codings come back as they are", matches!(get(&c, &stacked), Ok(ref x) if x.body_bytes() == b"raw" && x.headers.get("content-encoding") == Some("gzip, br")));
    t.check("decode: an unknown coding comes back as it is", matches!(get(&c, &with("compress", b"raw")), Ok(ref x) if x.body_bytes() == b"raw" && x.headers.get("content-encoding").is_some()));
    let mut h = Headers::new();
    h.set("Accept-Encoding", "gzip");
    let s = Scripted::new(&with("gzip", &gz));
    let r = c.request_on(&s, "GET", &url, &h, &[]);
    t.check("decode: the caller's own Accept-Encoding gets the raw body", matches!(r, Ok(ref x) if x.body_bytes() == &gz[..] && x.headers.get("content-encoding") == Some("gzip")));
    let r = c.request_on(&Scripted::new(b"HTTP/1.1 304 Not Modified\r\nContent-Encoding: gzip\r\n\r\n"), "GET", &url, &Headers::new(), &[]);
    t.check("decode: 304 keeps its headers", matches!(r, Ok(ref x) if x.status == 304 && x.headers.get("content-encoding") == Some("gzip")));
}

// ------------------------------------------------------------------ servers

/// a raw server: every request of a connection goes to the handler with the connection (it
/// writes the answer itself; false: the connection ends); counts the connections
struct Raw {
    port: u16,
    conns: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    th: Option<plat::Thread>,
}

type RawHandler = Arc<dyn Fn(&Request, &TcpStream) -> bool + Send + Sync>;

impl Raw {
    fn start(h: RawHandler) -> Option<Raw> {
        let l = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0)).ok()?;
        let port = l.local_addr().ok()?.port;
        let (conns, stop) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicBool::new(false)));
        let (c2, s2) = (conns.clone(), stop.clone());
        let th = plat::thread_spawn(0, Box::new(move || {
            while !s2.load(SeqCst) {
                let Ok(Some((s, _))) = l.accept_timeout(50) else { continue };
                c2.fetch_add(1, SeqCst);
                let (h, s3) = (h.clone(), s2.clone());
                let _ = plat::thread_spawn(0, Box::new(move || {
                    s.set_read_timeout(Some(3000));
                    s.set_write_timeout(Some(3000));
                    let mut rd = Reader::new(&s);
                    while !s3.load(SeqCst) {
                        let Ok((start, headers)) = rd.head() else { break };
                        let mut p = start.split(' ');
                        let (Some(m), Some(target)) = (p.next(), p.next()) else { break };
                        let body = if m == "CONNECT" { Vec::new() } else { rd.body(&headers, 1 << 20, false).unwrap_or_default() };
                        let mut r = Request::new(m, target);
                        r.headers = headers;
                        r.body = body;
                        if !h(&r, &s) {
                            break;
                        }
                    }
                    let _ = s.shutdown_write();
                }));
            }
        }));
        Some(Raw { port, conns, stop, th })
    }

    fn url(&self, p: &str) -> Url {
        u(&format!("http://127.0.0.1:{}{}", self.port, p))
    }

    fn conns(&self) -> u32 {
        self.conns.load(SeqCst)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        self.stop.store(true, SeqCst);
        if let Some(th) = self.th.take() {
            th.join();
        }
    }
}

fn w(s: &TcpStream, b: &[u8]) {
    let _ = s.write_all(b);
}

/// what the echo route says about a request: method, body length, and which headers it had
fn echo(r: &Request) -> String {
    let has = |n: &str| r.header(n).unwrap_or("-");
    format!("{} len={} ct={} auth={} cookie={} host={}", r.method, r.body.len(), has("content-type"), has("authorization"), has("cookie"), has("host"))
}

fn redirect_handler(other: u16) -> impl Fn(&Request) -> Response + Send + Sync + 'static {
    move |r: &Request| {
        let to = |status: u16, loc: &str| Response::new(status).header("Location", loc);
        let p = r.path();
        if let Some(n) = p.strip_prefix("/chain/").and_then(|n| n.parse::<u32>().ok()) {
            return if n == 0 { Response::text(200, "end") } else { to(301, &format!("/chain/{}", n - 1)) };
        }
        match p {
            "/loop" => to(302, "/loop"),
            "/to-echo" => to(302, "/echo"),
            "/to-other" => to(302, &format!("http://localhost:{}/echo", other)),
            "/see-other" => to(303, "/echo"),
            "/temp" => to(307, "/echo"),
            "/perm" => to(308, "/echo"),
            "/moved" => to(301, "/echo"),
            "/found" => to(302, "/echo"),
            "/rel/a/b" => to(302, "../c?x=1#frag"),
            "/rel/c" => Response::text(200, &format!("c {}", r.target)),
            "/bad-loc" => to(302, "http://[::1"),
            "/ftp-loc" => to(302, "ftp://example.org/x"),
            "/no-loc" => Response::new(302),
            "/echo" => Response::text(200, &echo(r)),
            "/set" => Response::text(200, "set").header("Set-Cookie", "s=1; Path=/"),
            _ => Response::text(404, "missing"),
        }
    }
}

fn start(h: impl Fn(&Request) -> Response + Send + Sync + 'static) -> Option<Server> {
    Server::start(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0), ServerConfig { threads: 4, ..ServerConfig::default() }, Arc::new(h)).ok()
}

fn redirects(t: &mut T) {
    // two origins: 127.0.0.1:a and localhost:b
    let Some(b) = start(redirect_handler(0)) else { return t.check("redirects: server", false) };
    let Some(a) = start(redirect_handler(b.local_addr().port)) else { return t.check("redirects: server", false) };
    let at = |p: &str| u(&format!("http://127.0.0.1:{}{}", a.local_addr().port, p));
    let plain = Client::new();
    t.check("redirects: off by default, the 3xx comes back", matches!(plain.get(&at("/chain/2"), &Headers::new()), Ok(ref r) if r.status == 301 && r.headers.get("location") == Some("/chain/1")));
    let c = Client::new().with_redirects(5);
    let f = c.fetch("GET", &at("/chain/3"), &Headers::new(), &[]);
    t.check("redirects: a chain of 3", matches!(f, Ok(ref f) if f.response.status == 200 && f.response.body_bytes() == b"end" && f.url.target == "/chain/0"));
    t.check("redirects: more than the limit", matches!(c.get(&at("/chain/6"), &Headers::new()), Err(HttpError::Redirect(_))));
    t.check("redirects: a loop", matches!(c.get(&at("/loop"), &Headers::new()), Err(HttpError::Redirect(_))));
    let mut h = Headers::new();
    h.set("Content-Type", "text/plain");
    let post = |p: &str| c.request("POST", &at(p), &h, b"abc").ok().map(|r| String::from_utf8_lossy(r.body_bytes()).into_owned());
    t.check("redirects: 303 after POST goes on as GET without the body", post("/see-other").is_some_and(|s| s.starts_with("GET len=0 ct=-")));
    t.check("redirects: 301 and 302 after POST go on as GET", post("/moved").is_some_and(|s| s.starts_with("GET len=0")) && post("/found").is_some_and(|s| s.starts_with("GET len=0")));
    t.check("redirects: 307 and 308 keep the method and the body", post("/temp").is_some_and(|s| s.starts_with("POST len=3 ct=text/plain")) && post("/perm").is_some_and(|s| s.starts_with("POST len=3")));
    let put = c.request("PUT", &at("/moved"), &h, b"abc").ok().map(|r| String::from_utf8_lossy(r.body_bytes()).into_owned());
    t.check("redirects: 301 after PUT keeps it", put.is_some_and(|s| s.starts_with("PUT len=3")));
    let mut cred = Headers::new();
    cred.set("Authorization", "Bearer secret");
    cred.set("Cookie", "session=1");
    let body = |p: &str| c.get(&at(p), &cred).ok().map(|r| String::from_utf8_lossy(r.body_bytes()).into_owned()).unwrap_or_default();
    let same = body("/to-echo");
    t.check(&format!("redirects: credentials go along within the origin ({same})"), same.contains("auth=Bearer secret") && same.contains("cookie=session=1"));
    let other = body("/to-other");
    t.check(&format!("redirects: to another origin, Authorization and Cookie stay behind ({other})"), other.starts_with("GET") && other.contains("auth=-") && other.contains("cookie=-") && other.contains(&format!("host=localhost:{}", b.local_addr().port)));
    let rel = c.fetch("GET", &at("/rel/a/b"), &Headers::new(), &[]);
    t.check("redirects: a relative Location (../c?x=1#frag)", matches!(rel, Ok(ref f) if f.response.body_bytes() == b"c /rel/c?x=1" && f.url.target == "/rel/c?x=1"));
    t.check("redirects: a Location that is not a URL", matches!(c.get(&at("/bad-loc"), &Headers::new()), Err(HttpError::Redirect(_))));
    t.check("redirects: a Location of another scheme", matches!(c.get(&at("/ftp-loc"), &Headers::new()), Err(HttpError::Redirect(_))));
    t.check("redirects: a 3xx without Location comes back", matches!(c.get(&at("/no-loc"), &Headers::new()), Ok(ref r) if r.status == 302));
    let checked = Client::new().with_redirects(5).with_redirect_check(Arc::new(|from: &Url, to: &Url| from.same_origin(to)));
    t.check("redirects: the check keeps a request within its origin", matches!(checked.get(&at("/to-other"), &Headers::new()), Ok(ref r) if r.status == 302) && matches!(checked.get(&at("/to-echo"), &Headers::new()), Ok(ref r) if r.status == 200));
    let mut heads = Vec::new();
    let r = c.get_to(&at("/chain/2"), &Headers::new(), 1 << 20, &mut |p| {
        if let Part::Head(s, _) = p {
            heads.push(s);
        }
        true
    });
    t.check("redirects: get_to sees only the last answer", r.is_ok_and(|(s, _)| s == 200) && heads == [200]);
    a.stop();
    b.stop();
}

fn https_redirects(t: &mut T) {
    let p = crate::tlst::pki(51);
    let scfg = netio::tls::ServerConfig::new(p.server.clone());
    let wrap: netio::server::Wrap = Arc::new(move |tcp: TcpStream| {
        let s = netio::tls::accept(Box::new(tcp), scfg.clone()).ok()?;
        Some((Box::new(s) as Box<dyn netio::Stream>, None))
    });
    let Some(plain) = start(|r: &Request| Response::text(200, &echo(r))) else { return t.check("https redirects: server", false) };
    let pp = plain.local_addr().port;
    let srv = Server::start_with(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0), ServerConfig { threads: 2, ..ServerConfig::default() }, Arc::new(move |r: &Request| match r.path() {
        "/down" => Response::new(302).header("Location", &format!("http://127.0.0.1:{}/echo", pp)),
        "/up" => Response::new(302).header("Location", "/fine"),
        _ => Response::text(200, "fine"),
    }), Some(wrap));
    let Ok(srv) = srv else { return t.check("https redirects: server", false) };
    let ca = p.ca.clone();
    let connector: netio::client::Connector = Arc::new(move |url: &Url, tcp: TcpStream| {
        let cfg = netio::tls::ClientConfig::new(netio::tls::Trust::Anchors(vec![ca.clone()]), &url.host);
        netio::tls::connect(Box::new(tcp), cfg).map(|s| Box::new(s) as Box<dyn netio::Stream>).map_err(|e| HttpError::Tls(format!("{}", e)))
    });
    let c = Client::new().with_connector(connector).with_redirects(3);
    let at = |p: &str| u(&format!("https://localhost:{}{}", srv.local_addr().port, p));
    t.check("https redirects: from https to http refused", matches!(c.get(&at("/down"), &Headers::new()), Err(HttpError::Redirect(ref s)) if s.contains("https to http")));
    t.check("https redirects: within https", matches!(c.get(&at("/up"), &Headers::new()), Ok(ref r) if r.body_bytes() == b"fine"));
    srv.stop();
    plain.stop();
}

// ------------------------------------------------------------------ cookies

fn cookies(t: &mut T) {
    use netpkt::psl;
    t.eq("psl: co.uk", psl::public_suffix("www.example.co.uk"), "co.uk");
    t.eq("psl: registrable domain", psl::registrable_domain("a.b.example.co.uk"), Some("example.co.uk"));
    t.check("psl: suffixes (com, co.uk, github.io, a wildcard)", ["com", "co.uk", "github.io", "foo.ck", "x.kobe.jp", "s3.amazonaws.com"].iter().all(|s| psl::is_public_suffix(s)));
    t.check("psl: not suffixes (an exception, names below one)", ["www.ck", "city.kobe.jp", "example.com", "user.github.io"].iter().all(|s| !psl::is_public_suffix(s)));
    t.eq("psl: an exception rule's site", psl::registrable_domain("www.ck"), Some("www.ck"));
    t.eq("psl: a name the list does not know: its last label", psl::public_suffix("host.example-unknown-tld"), "example-unknown-tld");
    t.check("psl: addresses and odd names have none", psl::public_suffix("192.168.1.1").is_empty() && psl::public_suffix("").is_empty() && psl::registrable_domain("com").is_none() && psl::public_suffix("a..b").is_empty());
    t.eq("psl: an IDN suffix as A-labels", psl::public_suffix("shop.xn--fiqs8s"), "xn--fiqs8s");
    t.check(&format!("psl: the list of {} ({})", psl::LIST_DATE, &psl::LIST_COMMIT[..12]), psl::LIST_COMMIT.len() == 40);
    let mut jar = Jar::new();
    let from = |h: &'static str| netpkt::cookie::Origin { host: h, path: "/", secure: true, http: true };
    t.check("cookie jar: Domain=github.io from a user's site refused (cookie tossing)", jar.store("x=1; Domain=github.io", from("evil.github.io"), 0).is_err());
    t.check("cookie jar: Domain=co.uk refused", jar.store("x=1; Domain=co.uk", from("shop.co.uk"), 0).is_err());
    t.check("cookie jar: Domain=example.co.uk from below it", jar.store("y=2; Domain=example.co.uk", from("a.example.co.uk"), 0).is_ok());
    t.check("cookie jar: a public suffix as the host's own name is host-only", jar.store("z=3; Domain=github.io", from("github.io"), 0).is_ok() && jar.cookies().iter().any(|c| c.name == "z" && c.host_only));
    t.eq("cookie jar: same site by the list", netpkt::cookie::site_of("a.b.example.co.uk"), String::from("example.co.uk"));
    t.check("cookie jar: a Domain with a dot at the end matches no host", jar.store("t=1; Domain=example.co.uk.", from("a.example.co.uk"), 0).is_err() && jar.store("t=1; Domain=github.io.", from("github.io"), 0).is_err());

    let Some(srv) = start(|r: &Request| match r.path() {
        "/set" => Response::text(200, "set").header("Set-Cookie", "s=1; Path=/"),
        "/set-and-go" => Response::new(302).header("Location", "/get").header("Set-Cookie", "r=3; Path=/"),
        "/get" => Response::text(200, r.header("cookie").unwrap_or("-")),
        _ => Response::text(404, ""),
    }) else {
        return t.check("cookies: server", false);
    };
    let at = |p: &str| u(&format!("http://127.0.0.1:{}{}", srv.local_addr().port, p));
    let jar = Arc::new(Mutex::new(Jar::new()));
    let c = Client::new().with_cookies(jar.clone()).with_redirects(3);
    let body = |c: &Client, p: &str, h: &Headers| c.get(&at(p), h).ok().map(|r| String::from_utf8_lossy(r.body_bytes()).into_owned()).unwrap_or_default();
    let _ = c.get(&at("/set"), &Headers::new());
    t.eq("cookies: stored and sent", body(&c, "/get", &Headers::new()), String::from("s=1"));
    t.eq("cookies: set on a redirect, sent on its target", body(&c, "/set-and-go", &Headers::new()), String::from("s=1; r=3"));
    let mut own = Headers::new();
    own.set("Cookie", "mine=9");
    t.eq("cookies: the caller's own Cookie header wins", body(&c, "/get", &own), String::from("mine=9"));
    let other = Client::new().with_cookies(jar.clone());
    t.eq("cookies: a client with the same jar", body(&other, "/get", &Headers::new()), String::from("s=1; r=3"));
    t.eq("cookies: none without a jar", body(&Client::new(), "/get", &Headers::new()), String::from("-"));
    t.check("cookies: the jar holds them", jar.lock().cookies().len() == 2);
    srv.stop();
}

// ------------------------------------------------------------------ the pool and Session

fn pool(t: &mut T) {
    let ok = |s: &TcpStream, body: &str, extra: &str| w(s, format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{}\r\n{}", body.len(), extra, body).as_bytes());
    let once_per_conn = Arc::new(Mutex::new(Vec::<u32>::new()));
    let seen = once_per_conn.clone();
    let Some(srv) = Raw::start(Arc::new(move |r: &Request, s: &TcpStream| match r.path() {
        "/a" => {
            ok(s, "a", "");
            true
        }
        "/close" => {
            ok(s, "c", "Connection: close\r\n");
            false
        }
        "/noclen" => {
            w(s, b"HTTP/1.1 200 OK\r\n\r\nxyz");
            false
        }
        "/tecl" => {
            w(s, b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 3\r\n\r\n1\r\nq\r\n0\r\n\r\n");
            true
        }
        "/extra" => {
            w(s, b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\naHTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nsmug!");
            true
        }
        "/http10" => {
            w(s, b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n10");
            true
        }
        "/bye" => {
            ok(s, "b", "");
            false // closed without saying so
        }
        "/once" => {
            // answers the first request of a connection, ends it without an answer at the second
            let mut v = seen.lock();
            v.push(1);
            if v.len() % 2 == 1 {
                ok(s, "1", "");
                true
            } else {
                false
            }
        }
        _ => false,
    })) else {
        return t.check("pool: server", false);
    };
    let p = Pool::new();
    let c = Client::new().with_pool(p.clone());
    let get = |c: &Client, path: &str| c.get(&srv.url(path), &Headers::new()).ok().map(|r| String::from_utf8_lossy(r.body_bytes()).into_owned());
    let n0 = srv.conns();
    let three = (0..3).all(|_| get(&c, "/a").as_deref() == Some("a"));
    t.check(&format!("pool: three requests over one connection ({} connections)", srv.conns() - n0), three && srv.conns() - n0 == 1 && p.idle() == 1);
    let step = |c: &Client, path: &str, want: &str, new: u32, what: &str, t: &mut T| {
        let n = srv.conns();
        let got = get(c, path);
        let a = get(c, "/a");
        t.check(&format!("pool: {what} ({:?}, {} new)", got, srv.conns() - n), got.as_deref() == Some(want) && a.as_deref() == Some("a") && srv.conns() - n == new);
    };
    step(&c, "/close", "c", 1, "Connection: close ends it", t);
    step(&c, "/noclen", "xyz", 1, "a body to the end of the connection ends it", t);
    step(&c, "/tecl", "q", 1, "Transfer-Encoding with Content-Length ends it", t);
    step(&c, "/extra", "a", 1, "bytes past an answer end it (no smuggled answer)", t);
    step(&c, "/http10", "10", 1, "an HTTP/1.0 answer without keep-alive ends it", t);
    let n = srv.conns();
    let _ = get(&c, "/bye");
    plat::sleep_ms(100);
    let after = get(&c, "/a");
    t.check(&format!("pool: a connection the server closed while idle is not used ({} new)", srv.conns() - n), after.as_deref() == Some("a") && srv.conns() - n == 1);
    p.clear();
    let n = srv.conns();
    let first = get(&c, "/once");
    let second = get(&c, "/once");
    t.check(&format!("pool: a kept connection closed at the next request: GET once more ({} new)", srv.conns() - n), first.as_deref() == Some("1") && second.as_deref() == Some("1") && srv.conns() - n == 2);
    p.clear();
    once_per_conn.lock().clear();
    let _ = get(&c, "/once");
    let post = c.request("POST", &srv.url("/once"), &Headers::new(), b"x");
    t.check("pool: ... but a POST is not sent twice", post.is_err());
    p.clear();
    let n = srv.conns();
    let plain = Client::new();
    let _ = (get(&plain, "/a"), get(&plain, "/a"));
    t.check("pool: without one, a connection per request", srv.conns() - n == 2);
    let other = Client::new().with_pool(p.clone()).with_connector(Arc::new(|_u: &Url, tcp: TcpStream| Ok(Box::new(tcp) as Box<dyn netio::Stream>)));
    let _ = get(&c, "/a");
    let n = srv.conns();
    let _ = get(&other, "/a");
    t.check("pool: another TLS layer does not take the connection", srv.conns() - n == 1);

    // Session: an answer without a length is read to the end, not taken as empty (the rest
    // would be the next answer)
    let mut sess = Session::new(Client::new(), srv.url("/"));
    let a = sess.request("GET", "/noclen", &Headers::new(), &[], 1 << 20);
    let b = sess.request("GET", "/a", &Headers::new(), &[], 1 << 20);
    t.check("session: an answer without a length, then the next", matches!(a, Ok(ref r) if r.body_bytes() == b"xyz") && matches!(b, Ok(ref r) if r.body_bytes() == b"a"));
    let a = sess.request("GET", "/extra", &Headers::new(), &[], 1 << 20);
    let b = sess.request("GET", "/a", &Headers::new(), &[], 1 << 20);
    t.check("session: bytes past an answer do not become the next one", matches!(a, Ok(ref r) if r.body_bytes() == b"a") && matches!(b, Ok(ref r) if r.body_bytes() == b"a"));
    drop(srv);
}

// ------------------------------------------------------------------ the deadline

fn deadline(t: &mut T) {
    let Some(srv) = Raw::start(Arc::new(|r: &Request, s: &TcpStream| {
        if r.path() == "/drip" {
            w(s, b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n");
        }
        for _ in 0..100 {
            let piece: &[u8] = if r.path() == "/drip" { b"x" } else { b"HTTP/1.1 100 Continue\r\n\r\n" };
            if s.write_all(piece).is_err() {
                break;
            }
            plat::sleep_ms(100);
        }
        false
    })) else {
        return t.check("deadline: server", false);
    };
    let mut c = Client::new();
    c.timeout_ms = 1000;
    c.deadline_ms = 500;
    let t0 = plat::time_us();
    let r = c.get(&srv.url("/drip"), &Headers::new());
    let ms = (plat::time_us() - t0) / 1000;
    t.check(&format!("deadline: a body that drips ends at the deadline ({ms} ms)"), matches!(r, Err(HttpError::Net(ref e)) if e.kind == NetErrorKind::TimedOut) && ms < 2000);
    let t0 = plat::time_us();
    let r = Client::new().get(&srv.url("/interim"), &Headers::new());
    let ms = (plat::time_us() - t0) / 1000;
    t.check(&format!("deadline: interim answers without end stop after 16 ({ms} ms)"), r.err() == Some(HttpError::Bad) && ms < 4000);
}

// ------------------------------------------------------------------ proxies at work

/// copy both ways between two connections until one ends
fn relay(a: &TcpStream, b: &TcpStream) {
    let mut buf = vec![0u8; 16 << 10];
    let t0 = plat::time_us();
    while plat::time_us() - t0 < 10_000_000 {
        let mut moved = false;
        for (from, to) in [(a, b), (b, a)] {
            if from.wait_readable(5).unwrap_or(false) {
                match from.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if to.write_all(&buf[..n]).is_err() {
                            return;
                        }
                        moved = true;
                    }
                }
            }
        }
        if !moved {
            plat::sleep_ms(1);
        }
    }
}

fn proxy_at_work(t: &mut T) {
    let Some(origin) = start(|r: &Request| Response::text(200, &format!("origin {}", echo(r)))) else { return t.check("proxy: origin", false) };
    let op = origin.local_addr().port;
    // the proxy: absolute-form requests answered itself, CONNECT to *.test:port relayed to
    // 127.0.0.1:port; "Basic dTpwdw==" (u:pw) required, "junk.test" answers with bytes after its 200
    let Some(px) = Raw::start(Arc::new(move |r: &Request, s: &TcpStream| {
        let auth = r.header("proxy-authorization") == Some("Basic dTpwdw==");
        if !auth {
            w(s, b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic\r\nContent-Length: 0\r\n\r\n");
            return true;
        }
        if r.method != "CONNECT" {
            let body = format!("via proxy: {} {} host={}", r.method, r.target, r.header("host").unwrap_or("-"));
            w(s, format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).as_bytes());
            return true;
        }
        let Some((host, port)) = r.target.rsplit_once(':') else { return false };
        if host == "junk.test" {
            w(s, b"HTTP/1.1 200 Connection established\r\n\r\nGARBAGE");
            return false;
        }
        let Ok(to) = TcpStream::connect(&SocketAddr::new(IpAddr::LOCALHOST_V4, port.parse().unwrap_or(0)), 2000) else { return false };
        w(s, b"HTTP/1.1 200 Connection established\r\n\r\n");
        relay(s, &to);
        false
    })) else {
        return t.check("proxy: server", false);
    };
    let with = |url: &str| Proxy::new(url).unwrap_or_default();
    let good = with(&format!("http://u:pw@127.0.0.1:{}", px.port));
    let c = Client::new().with_proxy(good.clone());
    let r = c.get(&u("http://no-such-host.invalid:81/x?y=1"), &Headers::new());
    t.check("proxy: http in absolute form, with credentials, no lookup of the host",
        matches!(r, Ok(ref r) if r.body_bytes() == b"via proxy: GET http://no-such-host.invalid:81/x?y=1 host=no-such-host.invalid:81"));
    // https through the tunnel; the "TLS layer" here hands the tunnel on as it is
    let pass: netio::client::Connector = Arc::new(|_u: &Url, tcp: TcpStream| Ok(Box::new(tcp) as Box<dyn netio::Stream>));
    let tc = Client::new().with_proxy(good.clone()).with_connector(pass.clone());
    let r = tc.get(&u(&format!("https://origin.test:{}/hello", op)), &Headers::new());
    t.check("proxy: a CONNECT tunnel to origin.test", matches!(r, Ok(ref r) if r.body_bytes().starts_with(b"origin GET len=0")));
    let bad = Client::new().with_proxy(with(&format!("http://u:wrong@127.0.0.1:{}", px.port))).with_connector(pass.clone());
    t.eq("proxy: wrong credentials (407)", bad.get(&u(&format!("https://origin.test:{}/", op)), &Headers::new()).err(), Some(HttpError::Proxy(407)));
    let r = tc.get(&u(&format!("https://junk.test:{}/", op)), &Headers::new());
    t.eq("proxy: bytes after the tunnel's 200 refused", r.err(), Some(HttpError::Bad));
    let direct = Client::new().with_proxy(good.clone());
    let r = direct.get(&u(&format!("http://127.0.0.1:{}/x", op)), &Headers::new());
    t.check("proxy: loopback goes direct", matches!(r, Ok(ref r) if r.body_bytes().starts_with(b"origin GET")));
    let p = Pool::new();
    let pooled = Client::new().with_proxy(good).with_pool(p.clone());
    let n = px.conns();
    let r1 = pooled.get(&u("http://a.example:81/1"), &Headers::new());
    let r2 = pooled.get(&u("http://a.example:81/2"), &Headers::new());
    t.check(&format!("proxy: kept connections to the proxy ({} new)", px.conns() - n), r1.is_ok() && r2.is_ok() && px.conns() - n == 1);
    origin.stop();
}

pub fn run(t: &mut T) {
    urls(t);
    proxies(t);
    answers(t);
    encoded(t);
    redirects(t);
    https_redirects(t);
    cookies(t);
    pool(t);
    deadline(t);
    proxy_at_work(t);
}

/// coretest --http-get URL [--proxy URL] [--redirects N] [--cookies]: one GET with netio's client
/// (a fresh jar with --cookies) -> "status final-url" on stdout, or the error and exit code 1
/// (tools/cookie_check.py runs the cookie tests of http-state through it)
pub fn get_mode(url: &str) -> i32 {
    let Some(url) = Url::parse(url) else {
        plat::write_stdout(b"error: not an http(s) URL\n");
        return 2;
    };
    let mut c = Client::new();
    if let Some(p) = plat::arg_value("--proxy") {
        let Some(p) = Proxy::new(&p) else {
            plat::write_stdout(b"error: not a proxy URL\n");
            return 2;
        };
        c = c.with_proxy(p);
    }
    if let Some(n) = plat::arg_value("--redirects").and_then(|n| n.parse().ok()) {
        c = c.with_redirects(n);
    }
    if plat::args().iter().any(|a| a == "--cookies") {
        c = c.with_cookies(Arc::new(Mutex::new(Jar::new())));
    }
    match c.fetch("GET", &url, &Headers::new(), &[]) {
        Ok(f) => {
            plat::write_stdout(format!("{} {}\n", f.response.status, f.url).as_bytes());
            0
        }
        Err(e) => {
            plat::write_stdout(format!("error: {}\n", e).as_bytes());
            1
        }
    }
}
