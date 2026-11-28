// platx -- the server-side parts of plat: sync (threads, channel), rand, net (loopback TCP /
// UDP over IPv4 and IPv6, Unix sockets, timeouts, refused connections), fs (private files, metadata, listing,
// links, times, free space), the local time zone; on Android sync and rand (AN1), net without Unix
// sockets (AN2: and the app without android.permission.INTERNET, android_net), fs, watch and the
// zone (AN3)

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::fs;
use plat::net::{IpAddr, NetErrorKind, SocketAddr, TcpListener, TcpStream, UdpSocket};
use plat::sync::{channel, Condvar, Mutex, RecvError, RwLock};

use crate::T;

pub fn run(t: &mut T) {
    // Android (docs/ANDROID.md): threads and sync since AN1, files, watch and the zone since AN3;
    // net waits for AN2, service for AN4
    #[cfg(target_os = "android")]
    {
        sync(t);
        rand(t);
        if android_net(t) {
            addrs(t);
            tcp(t, IpAddr::LOCALHOST_V4);
            tcp(t, IpAddr::LOCALHOST_V6);
            udp(t);
        }
        t.later("net: Unix sockets");
        files(t);
        watch(t);
        zone(t);
        t.later("service (AN4: Unsupported)");
    }
    #[cfg(not(target_os = "android"))]
    run_all(t);
}

#[cfg(not(target_os = "android"))]
fn run_all(t: &mut T) {
    service(t);
    sync(t);
    rand(t);
    addrs(t);
    tcp(t, IpAddr::LOCALHOST_V4);
    tcp(t, IpAddr::LOCALHOST_V6);
    udp(t);
    unix(t);
    files(t);
    watch(t);
    zone(t);
}

/// the local zone's name, one datetime knows (Android: the system's setting persist.sys.timezone)
fn zone(t: &mut T) {
    let z = fs::local_zone_id();
    t.check("local zone id", z.as_deref().is_some_and(|z| !z.is_empty()));
    t.check("local zone known to datetime", z.as_deref().and_then(zone_of).is_some());
}

fn zone_of(id: &str) -> Option<datetime::Zone> {
    match id.strip_prefix("win:") {
        Some(k) => datetime::Zone::from_windows(k),
        None => datetime::Zone::by_name(id),
    }
}

fn sync(t: &mut T) {
    let m = Arc::new(Mutex::new(0u64));
    let mut hs = Vec::new();
    for _ in 0..8 {
        let m = m.clone();
        hs.push(plat::thread_spawn(0, alloc::boxed::Box::new(move || {
            for _ in 0..20_000 {
                *m.lock() += 1;
            }
        })));
    }
    for h in hs.into_iter().flatten() {
        h.join();
    }
    t.eq("mutex counter", *m.lock(), 160_000);
    // condvar handshake
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let p2 = pair.clone();
    let h = plat::thread_spawn(0, alloc::boxed::Box::new(move || {
        plat::sleep_ms(20);
        *p2.0.lock() = true;
        p2.1.notify_all();
    }));
    let g = pair.1.wait_while(pair.0.lock(), |ready| !*ready);
    t.check("condvar", *g);
    drop(g);
    if let Some(h) = h {
        h.join();
    }
    let (g, timed_out) = pair.1.wait_timeout(pair.0.lock(), Some(30));
    drop(g);
    t.check("condvar timeout", timed_out);
    // rwlock
    let l = RwLock::new(5);
    {
        let a = l.read();
        let b = l.read();
        t.eq("rwlock readers", *a + *b, 10);
    }
    *l.write() += 1;
    t.eq("rwlock writer", *l.read(), 6);
    // channel: producers, bounded queue, disconnect
    let (tx, rx) = channel::<u32>(4);
    let mut hs = Vec::new();
    for k in 0..4u32 {
        let tx = tx.clone();
        hs.push(plat::thread_spawn(0, alloc::boxed::Box::new(move || {
            for i in 0..250 {
                let _ = tx.send(k * 1000 + i);
            }
        })));
    }
    drop(tx);
    let mut sum = 0u64;
    let mut n = 0;
    while let Some(v) = rx.recv() {
        sum += v as u64;
        n += 1;
    }
    for h in hs.into_iter().flatten() {
        h.join();
    }
    t.eq("channel count", n, 1000);
    t.eq("channel sum", sum, (0..4u64).map(|k| k * 1000 * 250 + 250 * 249 / 2).sum::<u64>());
    t.eq("channel disconnected", rx.recv_timeout(10), Err(RecvError::Disconnected));
    let (tx, rx) = channel::<u8>(1);
    t.eq("channel timeout", rx.recv_timeout(20), Err(RecvError::Timeout));
    t.check("channel try_send full", tx.try_send(1).is_ok() && tx.try_send(2).is_err());
    drop(rx);
    t.check("channel receiver gone", tx.send(3).is_err());
}

fn rand(t: &mut T) {
    let a: [u8; 32] = plat::random_bytes();
    let b: [u8; 32] = plat::random_bytes();
    t.check("random differs", a != b && a != [0; 32]);
    let mut big = alloc::vec![0u8; 1 << 16];
    plat::fill_random(&mut big);
    let ones: u32 = big.iter().map(|x| x.count_ones()).sum();
    // 524288 bits: a fair source stays well within +-1 %
    t.check("random balance", (ones as i64 - 262_144).abs() < 2600);
}

fn addrs(t: &mut T) {
    t.eq("parse v4", SocketAddr::parse("10.0.0.5:9443"), Some(SocketAddr::new(IpAddr::V4([10, 0, 0, 5]), 9443)));
    t.eq("parse v6", SocketAddr::parse("[::1]:80").map(|a| format!("{}", a)), Some(String::from("[::1]:80")));
    t.eq("v6 shortening", IpAddr::parse("fe80:0:0:0:0:0:0:1").map(|a| format!("{}", a)), Some(String::from("fe80::1")));
    t.eq("v6 mapped", IpAddr::parse("::ffff:192.168.1.1").and_then(|a| a.mapped_v4()), Some(IpAddr::V4([192, 168, 1, 1])));
    t.eq("v6 middle run", IpAddr::parse("2001:db8:0:0:1:0:0:1").map(|a| format!("{}", a)), Some(String::from("2001:db8::1:0:0:1")));
    t.check("bad v4", IpAddr::parse("1.2.3").is_none() && IpAddr::parse("1.2.3.256").is_none() && IpAddr::parse("01.2.3.4").is_none());
    t.check("bad v6", IpAddr::parse("1::2::3").is_none() && IpAddr::parse("12345::").is_none());
    t.check("v4 needs no brackets", SocketAddr::parse("::1:80").is_none());
    t.check("loopback", IpAddr::LOCALHOST_V4.is_loopback() && IpAddr::LOCALHOST_V6.is_loopback());
    let r = plat::net::resolve("localhost", 7);
    t.check("resolve localhost", r.is_ok_and(|v| v.iter().any(|a| a.ip.is_loopback())));
}

fn tcp(t: &mut T, ip: IpAddr) {
    let fam = if matches!(ip, IpAddr::V4(_)) { "v4" } else { "v6" };
    let Ok(l) = TcpListener::bind(&SocketAddr::new(ip, 0)) else {
        t.check(&format!("tcp {} bind", fam), false);
        return;
    };
    let addr = l.local_addr().unwrap_or(SocketAddr::new(ip, 0));
    t.check(&format!("tcp {} port", fam), addr.port != 0);
    let server = plat::thread_spawn(0, alloc::boxed::Box::new(move || {
        // echo server: one connection, echo until the peer shuts down
        if let Ok((s, _)) = l.accept() {
            let mut buf = [0u8; 4096];
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 || s.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }));
    let c = TcpStream::connect(&addr, 2000);
    t.check(&format!("tcp {} connect", fam), c.is_ok());
    if let Ok(c) = c {
        let _ = c.set_nodelay(true);
        c.set_read_timeout(Some(3000));
        t.check(&format!("tcp {} peer addr", fam), c.peer_addr().is_ok_and(|a| a == addr));
        let msg: Vec<u8> = (0..200_000u32).map(|i| (i * 31) as u8).collect();
        // write and read concurrently (the echo would block on a full buffer otherwise)
        let c = Arc::new(c);
        let w = c.clone();
        let m2 = msg.clone();
        let writer = plat::thread_spawn(0, alloc::boxed::Box::new(move || {
            let _ = w.write_all(&m2);
            let _ = w.shutdown_write();
        }));
        let mut got = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match c.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        if let Some(w) = writer {
            w.join();
        }
        t.check(&format!("tcp {} echo 200 KB", fam), got == msg);
    }
    if let Some(s) = server {
        s.join();
    }
    // a read timeout on a silent server
    if let Ok(l) = TcpListener::bind(&SocketAddr::new(ip, 0)) {
        let a = l.local_addr().unwrap_or(addr);
        if let Ok(c) = TcpStream::connect(&a, 2000) {
            c.set_read_timeout(Some(50));
            let mut b = [0u8; 4];
            let r = c.read(&mut b);
            t.check(&format!("tcp {} read timeout", fam), r.is_err_and(|e| e.kind == NetErrorKind::TimedOut));
            let r = l.accept_timeout(1000);
            t.check(&format!("tcp {} accept_timeout", fam), r.is_ok_and(|x| x.is_some()));
        }
        t.check(&format!("tcp {} accept times out", fam), l.accept_timeout(30).is_ok_and(|x| x.is_none()));
    }
    // nobody listens: refused
    let free = TcpListener::bind(&SocketAddr::new(ip, 0)).and_then(|l| l.local_addr());
    if let Ok(a) = free {
        let r = TcpStream::connect(&a, 2000);
        t.check(&format!("tcp {} refused", fam), r.is_err_and(|e| e.kind == NetErrorKind::Refused || e.kind == NetErrorKind::TimedOut));
    }
}

fn udp(t: &mut T) {
    let a = UdpSocket::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0));
    let b = UdpSocket::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0));
    let (Ok(a), Ok(b)) = (a, b) else {
        t.check("udp bind", false);
        return;
    };
    let ba = b.local_addr().unwrap_or(SocketAddr::new(IpAddr::LOCALHOST_V4, 0));
    t.check("udp send", a.send_to(b"ping", &ba).is_ok_and(|n| n == 4));
    let mut buf = [0u8; 64];
    let r = b.recv_from(&mut buf, 2000);
    t.check("udp receive", r.is_ok_and(|(n, from)| &buf[..n] == b"ping" && from == a.local_addr().unwrap_or(from)));
    t.check("udp timeout", b.recv_from(&mut buf, 30).is_err_and(|e| e.kind == NetErrorKind::TimedOut));
}

/// Unix domain sockets: bind, connect, echo both ways, timeouts, the socket file goes with the
/// listener, a missing socket is refused
#[cfg(not(target_os = "android"))]
fn unix(t: &mut T) {
    let path = fs::join(&plat::temp_dir().unwrap_or_else(|| String::from(".")), &format!("ct-{}.sock", plat::random_u64() % 1_000_000));
    let l = match plat::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            t.check(&format!("unix bind: {}", e), false);
            return;
        }
    };
    t.check("unix socket file", fs::lstat(&path).is_some());
    let p2 = path.clone();
    let th = plat::thread_spawn(64 * 1024, alloc::boxed::Box::new(move || {
        if let Ok(c) = plat::net::connect_unix(&p2, 2000) {
            let _ = c.write_all(b"hallo unix");
            let mut b = [0u8; 16];
            let _ = c.read_exact(&mut b[..4]);
            let _ = c.write_all(&b[..4]);
        }
    }));
    let got = match l.accept_timeout(3000) {
        Ok(Some(c)) => {
            let mut b = [0u8; 10];
            let r1 = c.read_exact(&mut b).is_ok() && &b == b"hallo unix";
            let r2 = c.write_all(b"echo").is_ok();
            let mut e = [0u8; 4];
            let r3 = c.read_exact(&mut e).is_ok() && &e == b"echo";
            c.set_read_timeout(Some(30));
            let r4 = c.read(&mut e).is_err_and(|x| x.kind == NetErrorKind::TimedOut) || c.read(&mut e).is_ok_and(|n| n == 0);
            r1 && r2 && r3 && r4
        }
        _ => false,
    };
    if let Some(h) = th {
        h.join();
    }
    t.check("unix echo both ways", got);
    t.check("unix accept times out", l.accept_timeout(30).is_ok_and(|x| x.is_none()));
    drop(l);
    t.check("unix socket file removed", fs::lstat(&path).is_none());
    t.check("unix refused", plat::net::connect_unix(&path, 500).is_err());
    unix_names(t, &path);
}

/// UnixListener and the names around it: bind never removes a file, a folder or a socket that
/// is listened on (an error), only a socket file left by a listener that is gone; a dropped
/// listener removes only its own socket file, not a name taken again meanwhile
#[cfg(not(target_os = "android"))]
fn unix_names(t: &mut T, path: &str) {
    use plat::net::{NetErrorKind, UnixListener};
    let in_use = |r: Result<UnixListener, plat::net::NetError>| r.err().is_some_and(|e| e.kind == NetErrorKind::AddrInUse);
    // a file and an empty folder at the name stay
    plat::write_file(path, b"payload");
    t.check("unix bind on a file: AddrInUse", in_use(UnixListener::bind(path)));
    t.check("unix ... the file keeps its bytes", plat::read_file(path).as_deref() == Some(&b"payload"[..]));
    plat::delete(path);
    plat::mkdir(path);
    // (Linux and macOS: AddrInUse; Windows: Other, WSAEINVAL)
    t.check("unix bind on an empty folder fails, the folder stays", UnixListener::bind(path).is_err() && plat::file_exists(path) == 2);
    plat::delete(path);
    // a listener's socket is not taken over
    let Ok(l1) = UnixListener::bind(path) else {
        t.check("unix bind", false);
        return;
    };
    t.check("unix second bind on a listened socket: AddrInUse", in_use(UnixListener::bind(path)));
    t.check("unix ... the first listener still reachable", plat::net::connect_unix(path, 2000).is_ok() && l1.accept_timeout(2000).is_ok_and(|c| c.is_some()));
    // the name deleted and taken again: dropping the old listener leaves the new file
    t.check("unix name taken again", plat::delete(path) && plat::write_file(path, b"neu"));
    drop(l1);
    t.check("unix a dropped listener leaves a name taken again", plat::read_file(path).as_deref() == Some(&b"neu"[..]));
    plat::delete(path);
    // a socket file left behind (its listener gone): bind takes the name
    let moved = format!("{path}.old");
    if let Ok(l2) = UnixListener::bind(path) {
        let mv = plat::rename(path, &moved);
        drop(l2); // its socket file is elsewhere now: not removed
        let left = mv && fs::lstat(&moved).is_some() && plat::rename(&moved, path);
        let l3 = UnixListener::bind(path);
        t.check("unix bind over a socket file nobody listens on", left && l3.is_ok());
        let reach = plat::net::connect_unix(path, 2000).is_ok();
        drop(l3);
        t.check("unix ... and listens there", reach && fs::lstat(path).is_none());
    }
    plat::delete(&moved);
}

/// plat::watch: new, changed, renamed and deleted files, a new folder and a file in it
fn watch(t: &mut T) {
    let base = fs::join(&plat::temp_dir().unwrap_or_else(|| String::from(".")), &format!("coretest-watch-{}", plat::random_u64() % 1_000_000));
    t.check("watch dir", fs::mkdir_private(&base) && fs::mkdir_private(&fs::join(&base, "sub")));
    plat::write_file(&fs::join(&base, "sub/alt.txt"), b"alt");
    let w = match plat::watch::Watcher::new(&[base.clone()]) {
        Ok(w) => w,
        Err(e) => {
            t.check(&format!("watcher: {}", e), false);
            return;
        }
    };
    plat::sleep_ms(100);
    t.check("watch quiet", w.take(0) == plat::watch::Changes::default());
    let ends = |c: &plat::watch::Changes, name: &str| c.paths.iter().any(|p| p.replace('\\', "/").ends_with(name));
    plat::write_file(&fs::join(&base, "neu.txt"), b"neu");
    plat::write_file(&fs::join(&base, "sub/alt.txt"), b"geaendert");
    let mut seen = plat::watch::Changes::default();
    for _ in 0..20 {
        let c = w.take(200);
        seen.paths.extend(c.paths);
        seen.overflow |= c.overflow;
        if ends(&seen, "/neu.txt") && ends(&seen, "/sub/alt.txt") {
            break;
        }
    }
    t.check("watch new file", ends(&seen, "/neu.txt"));
    t.check("watch changed file in a folder", ends(&seen, "/sub/alt.txt"));
    // a new folder, then a file in it (Linux: the folder gets its own watch)
    fs::mkdir_private(&fs::join(&base, "neu"));
    plat::sleep_ms(150);
    plat::write_file(&fs::join(&base, "neu/innen.txt"), b"x");
    plat::rename(&fs::join(&base, "neu.txt"), &fs::join(&base, "umbenannt.txt"));
    plat::delete(&fs::join(&base, "sub/alt.txt"));
    let mut seen = plat::watch::Changes::default();
    for _ in 0..20 {
        let c = w.take(200);
        seen.paths.extend(c.paths);
        if ends(&seen, "/neu/innen.txt") && ends(&seen, "/umbenannt.txt") && ends(&seen, "/sub/alt.txt") {
            break;
        }
    }
    t.check("watch file in a new folder", ends(&seen, "/neu/innen.txt"));
    t.check("watch rename", ends(&seen, "/umbenannt.txt"));
    t.check("watch delete", ends(&seen, "/sub/alt.txt"));
    drop(w);
    for p in ["neu/innen.txt", "neu", "umbenannt.txt", "sub", ""] {
        plat::delete(&fs::join(&base, p));
    }
    plat::delete(&base);
    watch_gaps(t);
}

/// true once cond holds (polled every 50 ms), false after ms
fn wait_until(ms: u64, mut cond: impl FnMut() -> bool) -> bool {
    let dl = plat::time_us() + ms * 1000;
    loop {
        if cond() {
            return true;
        }
        if plat::time_us() >= dl {
            return false;
        }
        plat::sleep_ms(50);
    }
}

/// a change whose path ends with suffix comes within ms
fn seen_within(w: &plat::watch::Watcher, suffix: &str, ms: u64) -> bool {
    wait_until(ms, || w.take(100).paths.iter().any(|p| p.replace('\\', "/").ends_with(suffix)))
}

/// the watcher's gaps: a change right after new, a root deleted and made again, (Linux, Android)
/// a root moved away, a folder whose listing failed
fn watch_gaps(t: &mut T) {
    use plat::watch::Watcher;
    let base = fs::join(&plat::temp_dir().unwrap_or_else(|| String::from(".")), &format!("coretest-watch2-{}", plat::random_u64() % 1_000_000));
    t.check("watch2 dir", fs::mkdir_private(&base));
    // every change after new returned is seen (Windows: new waits for each root's first read)
    let mut missed = 0;
    for i in 0..10 {
        let Ok(w) = Watcher::new(&[base.clone()]) else { missed += 1; continue };
        let f = fs::join(&base, &format!("start{i}.txt"));
        plat::write_file(&f, b"x");
        if !seen_within(&w, &format!("/start{i}.txt"), 3000) {
            missed += 1;
        }
        drop(w);
        plat::delete(&f);
    }
    t.check("watch: a change right after new is seen (10 watchers)", missed == 0);
    #[cfg(any(windows, target_os = "linux", target_os = "android"))]
    {
        // a root deleted: lost (overflow at every take, watching() false); made again: watched
        // again, its changes are seen
        let root = fs::join(&base, "wurzel");
        fs::mkdir_private(&root);
        let Ok(w) = Watcher::new(&[root.clone()]) else {
            t.check("watch: a watcher of one root", false);
            return;
        };
        t.check("watch: watching() after new", w.watching());
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            // (Linux) moved away: its changes there are not reported under the old path; a new
            // folder at the path is watched
            let moved = fs::join(&base, "verschoben");
            t.check("watch: the root moved away", plat::rename(&root, &moved) && wait_until(3000, || !w.watching()));
            plat::write_file(&fs::join(&moved, "alt.txt"), b"x");
            t.check("watch: ... its changes there are not reported under the old path", !seen_within(&w, "/wurzel/alt.txt", 1000));
            fs::mkdir_private(&root);
            t.check("watch: a new folder at the root's path is watched", wait_until(5000, || w.watching()));
            plat::write_file(&fs::join(&root, "neu.txt"), b"x");
            t.check("watch: ... a change in it is seen", seen_within(&w, "/wurzel/neu.txt", 3000));
            plat::delete(&fs::join(&moved, "alt.txt"));
            plat::delete(&moved);
            plat::delete(&fs::join(&root, "neu.txt"));
        }
        plat::delete(&root);
        t.check("watch: a deleted root is lost (overflow, watching() false)", wait_until(3000, || w.take(100).overflow && !w.watching()));
        fs::mkdir_private(&root);
        t.check("watch: the root made again is watched again", wait_until(5000, || w.watching()));
        plat::write_file(&fs::join(&root, "danach.txt"), b"x");
        t.check("watch: ... a change in it is seen", seen_within(&w, "/wurzel/danach.txt", 3000));
        drop(w);
        plat::delete(&fs::join(&root, "danach.txt"));
        plat::delete(&fs::join(&root, "neu.txt"));
        plat::delete(&root);
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // the first listing of a folder fails: its folders are watched a second later
        let list = fs::join(&base, "liste");
        let deep = fs::join(&list, "a/b");
        t.check("watch: a tree", fs::mkdir_private(&deep));
        plat::setenv("jfgui_TEST_WATCH_LIST_FAIL", Some(&list));
        let w = Watcher::new(&[list.clone()]);
        plat::setenv("jfgui_TEST_WATCH_LIST_FAIL", None);
        if let Ok(w) = w {
            t.check("watch: a folder whose listing failed: not all watched yet", !w.watching() && w.take(0).overflow);
            t.check("watch: ... a second later it is", wait_until(5000, || w.watching()));
            plat::write_file(&fs::join(&deep, "x.txt"), b"x");
            t.check("watch: ... a change two folders down is seen", seen_within(&w, "/liste/a/b/x.txt", 3000));
        } else {
            t.check("watch: a watcher of the tree", false);
        }
        plat::delete(&fs::join(&deep, "x.txt"));
        plat::delete(&deep);
        plat::delete(&fs::join(&list, "a"));
        plat::delete(&list);
    }
    plat::delete(&base);
}

fn files(t: &mut T) {
    let base = fs::join(&plat::temp_dir().unwrap_or_else(|| String::from(".")), &format!("coretest-{}", plat::random_u64() % 1_000_000));
    t.check("mkdir_private", fs::mkdir_private(&base));
    let f = fs::join(&base, "secret.key");
    t.check("write_file_private", fs::write_file_private(&f, b"0123456789"));
    t.eq("private file content", plat::read_file(&f).as_deref(), Some(&b"0123456789"[..]));
    let m = fs::stat(&f);
    t.check("stat file", m.is_some_and(|m| m.is_file() && m.size == 10 && m.ino != 0 && m.nlink >= 1));
    #[cfg(any(target_os = "linux", target_os = "android"))]
    t.check("private mode", m.is_some_and(|m| m.mode & 0o777 == 0o600));
    t.check("stat dir", fs::stat(&base).is_some_and(|m| m.is_dir()));
    // times
    let when = 1_700_000_000_123_456_700i64;
    t.check("set_times", fs::set_times(&f, when, when));
    t.check("mtime kept", fs::stat(&f).is_some_and(|m| m.mtime_ns == when));
    // hard link: same file id (Android: none, SELinux refuses apps every hard link)
    let h = fs::join(&base, "hard");
    if cfg!(target_os = "android") {
        t.check("hard link: none on Android, nothing made", !fs::hard_link(&f, &h) && fs::lstat(&h).is_none());
    } else if fs::hard_link(&f, &h) {
        let (a, b) = (fs::stat(&f), fs::stat(&h));
        t.check("hard link id", a.is_some_and(|a| b.is_some_and(|b| a.ino == b.ino && a.dev == b.dev && b.nlink == 2)));
    } else {
        t.check("hard link", false);
    }
    // symlink (Windows needs developer mode: skipped when refused)
    let s = fs::join(&base, "link");
    if fs::symlink("secret.key", &s, false) {
        t.check("lstat symlink", fs::lstat(&s).is_some_and(|m| m.is_symlink()));
        t.check("stat follows", fs::stat(&s).is_some_and(|m| m.is_file() && m.size == 10));
        t.eq("read_link", fs::read_link(&s), Some(String::from("secret.key")));
    }
    // listing
    plat::mkdir(&fs::join(&base, "sub"));
    let mut names: Vec<(String, fs::FileKind)> = fs::read_dir(&base).unwrap_or_default().into_iter().map(|e| (e.name, e.kind)).collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    t.check("read_dir", names.iter().any(|(n, k)| n == "sub" && *k == fs::FileKind::Dir) && names.iter().any(|(n, k)| n == "secret.key" && *k == fs::FileKind::File));
    t.check("free_space", fs::free_space(&base).is_some_and(|(free, total)| total > 0 && free <= total));
    t.check("sync_dir", fs::sync_dir(&base));
    // a long path (more than 260 characters) on every platform
    let mut deep = base.clone();
    for i in 0..12 {
        deep = fs::join(&deep, &format!("verzeichnis-mit-langem-namen-{:02}", i));
    }
    let long_ok = fs::mkdir_private(&deep) && fs::write_file_private(&fs::join(&deep, "datei.bin"), b"x");
    t.check("long paths", long_ok && fs::stat(&fs::join(&deep, "datei.bin")).is_some_and(|m| m.size == 1));
    t.eq("parent", fs::parent("/a/b/c"), "/a/b");
    names_and_siblings(t, &base);
    // clean up (best effort)
    remove_tree(&base);
}

/// '\' separates on Windows, on Android (as in its names) and in vfs names; on Linux and macOS it
/// is part of a name. The private writer's sibling is always a new file: a file of that name stays
/// as it was.
fn names_and_siblings(t: &mut T, base: &str) {
    t.eq("parent of a vfs name with a backslash", fs::parent("mem://a\\b"), "mem://a");
    if cfg!(windows) {
        t.eq("join after a backslash", fs::join("C:\\a\\", "f"), String::from("C:\\a\\f"));
        t.eq("parent across a backslash", fs::parent("C:\\a\\b"), "C:\\a");
    } else if cfg!(target_os = "android") {
        t.eq("join after a backslash (Android: a separator)", fs::join("/tmp/a\\", "f"), String::from("/tmp/a\\f"));
        t.eq("parent across a backslash (Android)", fs::parent("/tmp/a\\b"), "/tmp/a");
        let xy = fs::join(base, "x\\y");
        t.check("mkdir_private of x\\y makes the folder y in x (Android)",
                fs::mkdir_private(&xy) && plat::file_exists(&fs::join(base, "x")) == 2 && fs::stat(&fs::join(base, "x/y")).is_some_and(|m| m.is_dir()));
    } else {
        t.eq("join after a backslash in a name", fs::join("/tmp/a\\", "f"), String::from("/tmp/a\\/f"));
        t.eq("parent of a name with a backslash", fs::parent("/tmp/a\\b"), "/tmp");
        let xy = fs::join(base, "x\\y");
        t.check("mkdir_private of a name with a backslash makes that folder alone",
                fs::mkdir_private(&xy) && plat::file_exists(&fs::join(base, "x")) == 0 && fs::stat(&xy).is_some_and(|m| m.is_dir()));
        let d = fs::join(base, "d\\");
        let inner = fs::join(&d, "in.txt");
        t.check("a file in a folder whose name ends with a backslash", plat::mkdir(&d) && fs::write_file_private(&inner, b"in")
                && fs::read_dir(&d).is_some_and(|l| l.len() == 1 && l[0].name == "in.txt" && l[0].kind == fs::FileKind::File));
    }
    // a file where the private writer's first sibling would go: it keeps its bytes and its name
    let p = fs::join(base, "seed.key");
    let planted = format!("{p}.tmp424242");
    plat::setenv("jfgui_TEST_PRIVATE_TMP", Some("424242"));
    let wrote = plat::write_file(&planted, b"planted") && fs::write_file_private(&p, b"secret");
    plat::setenv("jfgui_TEST_PRIVATE_TMP", None);
    t.check("write_file_private takes no existing sibling over", wrote && plat::read_file(&p).as_deref() == Some(&b"secret"[..])
            && plat::read_file(&planted).as_deref() == Some(&b"planted"[..]));
    plat::delete(&planted);
}

fn remove_tree(path: &str) {
    if let Some(list) = fs::read_dir(path) {
        for e in list {
            let p = fs::join(path, &e.name);
            if e.kind == fs::FileKind::Dir {
                remove_tree(&p);
            } else {
                plat::delete(&fs::long_path(&p));
            }
        }
    }
    plat::delete(&fs::long_path(path));
}

/// the systemd unit of a service: quoting of the command line, the fixed settings
#[cfg(not(target_os = "android"))]
fn service(t: &mut T) {
    let args = [String::from("run"), String::from("--data"), String::from("/srv/backup data"), String::from("50%"), String::from("$HOME")];
    let spec = plat::service::ServiceSpec { name: "x", display: "X", description: "Ein Dienst", exe: "/opt/x/bin", args: &args };
    let u = plat::service::unit_text(&spec);
    t.check("unit: ExecStart quoted", u.contains("ExecStart=/opt/x/bin run --data \"/srv/backup data\" 50%% $$HOME\n"));
    t.check("unit: restart and install", u.contains("Restart=on-failure") && u.contains("WantedBy=multi-user.target") && u.contains("Description=Ein Dienst"));
    // a \r (systemd ends a line there too) or another control character cannot add a directive
    let spec = plat::service::ServiceSpec { name: "x", display: "X", description: "Ein\rExecStartPre=/bin/evil\n\tDienst", exe: "/opt/x/bin", args: &[] };
    let u = plat::service::unit_text(&spec);
    t.check("unit: control characters in the description are spaces", u.contains("Description=Ein ExecStartPre=/bin/evil  Dienst\n") && !u.contains('\r'));
    t.check("not a service: no stop request", !plat::service::stop_requested());
}

/// Android (docs/ANDROID.md AN2) -> whether the app may use the network.  With
/// android.permission.INTERNET: getaddrinfo (Bionic, netd) through the system's hosts file, the
/// device's interfaces and addresses without netlink, its name.  Without it every IPv4 and IPv6
/// socket and every lookup is refused with NetError::no_internet and a word on the manifest; the
/// line "plat: no network: ..." shows it (tools/android_console.py --no-internet expects it)
#[cfg(target_os = "android")]
fn android_net(t: &mut T) -> bool {
    use plat::net::{if_index, local_addrs, resolve, NetError};
    let any4 = SocketAddr::new(IpAddr::LOCALHOST_V4, 0);
    if let Err(e) = UdpSocket::bind(&any4) {
        if !e.no_internet() {
            t.check(&format!("net: a UDP socket ({})", e), false);
            return false;
        }
        plat::write_stdout(format!("plat: no network: {}\n", e).as_bytes());
        let text = format!("{}", e);
        t.check("no INTERNET: the message names the manifest's permission", text.contains("android.permission.INTERNET") && e.kind == NetErrorKind::Denied);
        let none = |r: Result<(), NetError>| r.is_err_and(|e| e.no_internet());
        t.check("no INTERNET: TCP listener", none(TcpListener::bind(&any4).map(|_| ())));
        t.check("no INTERNET: TCP connection", none(TcpStream::connect(&SocketAddr::new(IpAddr::LOCALHOST_V4, 9), 2000).map(|_| ())));
        t.check("no INTERNET: IPv6", none(UdpSocket::bind(&SocketAddr::new(IpAddr::LOCALHOST_V6, 0)).map(|_| ())));
        t.check("no INTERNET: a name through the system's resolver", none(resolve("example.org", 443).map(|_| ())));
        // numbers and localhost need no lookup
        t.check("no INTERNET: localhost still resolves", resolve("localhost", 80).is_ok());
        return false;
    }
    // Android's /system/etc/hosts names ::1 ip6-localhost: getaddrinfo without a name server
    let r = resolve("ip6-localhost", 80);
    t.check(&format!("getaddrinfo: ip6-localhost through the system's hosts file ({:?})", r), r.as_ref().is_ok_and(|v| v.contains(&SocketAddr::new(IpAddr::LOCALHOST_V6, 80))));
    let r = resolve("no-such-name.invalid", 80);
    t.check(&format!("getaddrinfo: an unknown name -> NotFound ({:?})", r), r.is_err_and(|e| e.kind == NetErrorKind::NotFound));
    let addrs = local_addrs();
    t.check(&format!("local addresses: loopback among them ({:?})", addrs), addrs.iter().any(|a| a.ip == IpAddr::LOCALHOST_V4));
    plat::write_stdout(format!("plat: local addresses {}\n", addrs.iter().map(|a| format!("{}", a.ip)).collect::<Vec<_>>().join(" ")).as_bytes());
    t.check("if_index lo", if_index("lo").is_some_and(|i| i > 0));
    t.check("hostname", !plat::net::hostname().is_empty());
    t.check("dns_servers: netd keeps them", plat::net::dns_servers().is_empty());
    true
}
