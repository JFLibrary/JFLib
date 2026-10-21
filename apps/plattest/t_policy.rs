// t_policy -- plattest --test policy: the app's permissions (lib/plat/policy.rs, guarded.rs)
//
//   1. normalize_for with the Windows and the Linux rules (both on every OS)
//   2. patterns (pattern_matches, this OS) and refused manifests (install fails, nothing is
//      installed)
//   3. a manifest for a scratch tree, installed in this process: reads, writes, traversal,
//      fs.rs, locks, a grant of the user's file, Key::trusted, processes, URLs, plugins,
//      print, secrets, autostart, capabilities(), the network (connect, listen, UDP, names, resolved addresses,
//      DNS, Unix sockets) and the denial records
// A policy stays installed until the process ends: this test is not part of "all".

use alloc::format;
use alloc::vec::Vec;

use plat::net::{IpAddr, NetErrorKind, SocketAddr, TcpListener, TcpStream, UdpSocket};
use plat::policy::{self, FS_READ};
use plat::{FO_READ, FO_WRITE, File, FileLock};

use crate::{T, rm_tree};

pub fn t_policy(t: &mut T) {
    normalize_rules(t);
    patterns(t);
    refused_manifests(t);
    installed(t);
}

fn normalize_rules(t: &mut T) {
    let win: [(&str, Option<&str>); 20] = [
        ("C:\\a\\b", Some("C:/a/b")),
        ("c:/a/./b/../c", Some("C:/a/c")),
        ("rel/x", Some("C:/work/rel/x")),
        ("/x", Some("C:/x")),
        ("C:", Some("C:/")),
        ("C:x", None),
        ("\\\\?\\C:\\a", Some("C:/a")),
        ("\\\\?\\UNC\\srv\\sh\\a", Some("//srv/sh/a")),
        ("\\\\srv\\sh\\a\\..\\b", Some("//srv/sh/b")),
        ("\\\\srv\\sh\\..\\x", None),
        ("\\\\.\\PhysicalDrive0", None),
        ("C:/a/file.txt:ads", None),
        ("C:/a/CON", None),
        ("C:/a/nul.txt", None),
        ("C:/a/b. ", Some("C:/a/b")),
        ("C:/a/.../b", None),
        ("C:/a/b*", None),
        ("C:/../x", None),
        ("", None),
        ("C:/a/com1/x", None),
    ];
    for (p, want) in win {
        let got = policy::normalize_for(p, true, "C:\\work");
        t.expect_s(got.as_deref(), want, &format!("policy: normalize (Windows) {p:?}"));
    }
    let lin: [(&str, Option<&str>); 8] = [
        ("/a//b/./c", Some("/a/b/c")),
        ("x/y", Some("/home/u/x/y")),
        ("/a/../../x", None),
        ("/a/b/../c", None),
        ("/", Some("/")),
        ("a\\b", Some("/home/u/a\\b")),
        ("/a/b:c", Some("/a/b:c")),
        ("/a/\0b", None),
    ];
    for (p, want) in lin {
        let got = policy::normalize_for(p, false, "/home/u");
        t.expect_s(got.as_deref(), want, &format!("policy: normalize (Linux) {p:?}"));
    }
}

fn patterns(t: &mut T) {
    let cases: Vec<(&str, &str, bool)> = if cfg!(windows) {
        alloc::vec![
            ("C:/Data/**", "c:\\data\\x\\y.txt", true),
            ("C:/Data/**", "C:/Data", true),
            ("C:/Data/**", "C:/DataX/y", false),
            ("C:/Data/*.png", "C:/data/a.PNG", true),
            ("C:/Data/*.png", "C:/Data/sub/a.png", false),
            ("C:/Data/**/b", "C:/Data/b", true),
            ("C:/Data/**/b", "C:/Data/x/y/b", true),
            ("C:/Data/a?c", "C:/Data/abc", true),
            ("C:/Data/**", "C:/Data/../Other", false),
            ("/usr/**", "C:/usr/x", false), // a Linux pattern: ignored on Windows
            ("**", "D:/anything", true),
        ]
    } else {
        alloc::vec![
            ("/data/**", "/data/x/y", true),
            ("/data/**", "/data", true),
            ("/data/**", "/datax", false),
            ("/data/*.png", "/data/a.png", true),
            ("/data/*.png", "/data/A.PNG", false),
            ("/data/*.png", "/data/s/a.png", false),
            ("/data/**/b", "/data/b", true),
            ("/data/**/b", "/data/x/y/b", true),
            ("/data/**", "/data/../etc/passwd", false),
            ("C:/data/**", "/data/x", false), // a Windows pattern: ignored on Linux
            ("**", "/anything", true),
        ]
    };
    for (p, path, want) in cases {
        t.check(policy::pattern_matches(p, path) == want, &format!("policy: {p} ~ {path} = {want}"));
    }
}

fn refused_manifests(t: &mut T) {
    for m in [
        "fs.read relative/path", "bogus", "url ftp:x", "fs.read $NOPE/x", "print now", "secrets all", "autostart on", "fs.write",
        "net.connect example.org", "net.connect ::1:80", "net.listen example.org:80", "net.connect 10.0.0.0/33:1",
        "net.connect a..b:1", "net.connect host:70000", "net.connect host:9-3", "net.connect [10.0.0.1]:1", "net.connect",
    ] {
        let r = policy::install(m);
        t.check(r.is_err(), &format!("policy: manifest {m:?} refused"));
    }
    t.check(!policy::active(), "policy: a refused manifest installs nothing");
}

fn installed(t: &mut T) {
    // the scratch tree, made before the policy is installed
    let cwd = plat::cwd();
    let base = format!("{}/build/plattest/{}-policy-{:x}", cwd.replace('\\', "/"), crate::OS, plat::time_us() & 0xFF_FFFF);
    let p = |s: &str| format!("{base}/{s}");
    let ok = plat::mkdir(&p("ro")) && plat::mkdir(&p("rw")) && plat::mkdir(&p("no"))
        && plat::write_file(&p("ro/a.txt"), b"ro") && plat::write_file(&p("no/secret.txt"), b"secret")
        && plat::write_file(&p("no/other.txt"), b"other") && plat::write_file(&p("no/user.txt"), b"user");
    t.check(ok, "policy: scratch tree");
    // a listener on a free loopback port, opened before the policy: the one port it allows
    let listener = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0)).ok();
    let port = listener.as_ref().and_then(|l| l.local_addr().ok()).map_or(0, |a| a.port);
    t.check(port != 0, "policy: a loopback listener before the policy");
    let manifest = format!(
        "# plattest --test policy\nfs.read {base}/ro/**\nfs.write {base}/rw/**   # writes imply reads\nfs.user\n\
         process plattest-no-such-program\nurl https MailTo\n\
         net.connect 127.0.0.1:{port} *.example.test:443 printer.lan:631-632\nnet.listen 127.0.0.1:0\n"
    );
    let key = match policy::install(&manifest) {
        Ok(k) => k,
        Err(e) => {
            t.check(false, &format!("policy: install ({e})"));
            return;
        }
    };
    t.check(policy::active(), "policy: active after install");
    t.check(policy::install("print").is_err(), "policy: a second install is refused");

    // files
    t.check(plat::read_file(&p("ro/a.txt")).as_deref() == Some(&b"ro"[..]), "policy: read below fs.read");
    t.check(plat::read_file(&p("ro/./a.txt")).is_some(), "policy: read through .");
    // Windows applies ".." to the text, Linux after symbolic links: there it is refused
    let up = plat::read_file(&p("ro/../ro/a.txt")).is_some();
    t.check(up == cfg!(windows), "policy: .. allowed on Windows, refused on Linux");
    t.check(!plat::write_file(&p("ro/b.txt"), b"x"), "policy: no write below fs.read");
    t.check(plat::write_file(&p("rw/c.txt"), b"rw"), "policy: write below fs.write");
    t.check(plat::read_file(&p("rw/c.txt")).as_deref() == Some(&b"rw"[..]), "policy: fs.write implies reading");
    t.check(plat::mkdir(&p("rw/sub/deeper")), "policy: mkdir below fs.write");
    t.check(plat::read_file(&p("no/secret.txt")).is_none(), "policy: read outside the rules refused");
    t.check(plat::read_file(&p("rw/../no/secret.txt")).is_none(), "policy: traversal out of fs.write refused");
    t.expect_i(plat::file_exists(&p("no/secret.txt")) as i64, 0, "policy: a refused file looks missing");
    t.check(plat::file_exists_checked(&p("no/secret.txt")).is_none(), "policy: file_exists_checked refused");
    t.check(!plat::dir_list(&p("no"), &mut |_, _, _| {}), "policy: dir_list outside refused");
    let mut n = 0;
    t.check(plat::dir_list(&p("ro"), &mut |_, _, _| n += 1) && n == 1, "policy: dir_list below fs.read");
    t.check(!plat::delete(&p("no/secret.txt")), "policy: delete outside refused");
    t.check(!plat::rename(&p("rw/c.txt"), &p("no/c.txt")), "policy: rename out of fs.write refused");
    t.check(File::open(&p("no/secret.txt"), FO_READ).is_none(), "policy: File::open outside refused");
    t.check(File::open(&p("ro/a.txt"), FO_WRITE).is_none(), "policy: File::open for writing below fs.read refused");
    t.check(File::open(&p("rw/d.txt"), FO_WRITE).is_some(), "policy: File::open for writing below fs.write");
    t.check(File::open_temp(Some(&p("no"))).is_none(), "policy: a temp file in a refused folder");
    t.check(File::open_temp(None).is_some(), "policy: an anonymous temp file");
    t.check(FileLock::acquire(&p("rw/LOCK")).is_some(), "policy: lock below fs.write");
    t.check(FileLock::acquire(&p("ro/LOCK")).is_none(), "policy: lock below fs.read refused");
    t.check(plat::fs::stat(&p("ro/a.txt")).is_some(), "policy: fs::stat below fs.read");
    t.check(plat::fs::stat(&p("no/secret.txt")).is_none(), "policy: fs::stat outside refused");
    t.check(plat::fs::read_dir(&p("no")).is_none(), "policy: fs::read_dir outside refused");
    t.check(!plat::fs::write_file_private(&p("no/x.txt"), b"x"), "policy: fs::write_file_private outside refused");
    if cfg!(windows) {
        t.check(plat::read_file(&p("RO/A.TXT")).is_some(), "policy: Windows compares without case");
        t.check(plat::read_file(&format!("{}:stream", p("ro/a.txt"))).is_none(), "policy: a stream name is refused");
    }

    // the user's file and trusted library code
    t.check(key.grant(&p("no/user.txt"), FS_READ), "policy: grant of a user file (fs.user)");
    t.check(plat::read_file(&p("no/user.txt")).is_some(), "policy: a granted file can be read");
    t.check(!plat::write_file(&p("no/user.txt"), b"x"), "policy: a read grant does not write");
    t.check(key.trusted(|| plat::read_file(&p("no/other.txt"))).is_some(), "policy: trusted code reads");
    t.check(plat::read_file(&p("no/other.txt")).is_none(), "policy: the trust ends with the call");

    // processes, URLs, plugins, print
    let cmd = plat::process::Command { program: "plattest-other-program", args: &[], cwd: None, env: &[] };
    let e = cmd.spawn().err();
    t.check(e == Some(if cfg!(windows) { 5 } else { 13 }), "policy: a process outside the rules is refused");
    t.check(policy::process_allowed("plattest-no-such-program"), "policy: a listed program name");
    t.check(policy::url_allowed("https://example.org") && policy::url_allowed("mailto:a@b"), "policy: listed URL schemes");
    t.expect_i(plat::open_url("ftp://example.org") as i64, -1, "policy: open_url of another scheme refused");
    t.check(plat::lib_open(&p("ro/plugin.dll")).is_err(), "policy: lib_open without plugins refused");
    t.check(!policy::print_allowed(), "policy: print without the permission refused");
    use plat::secret::{SecretError, secret_available, secret_get_checked, secret_set};
    t.check(secret_set("plattest-policy", "a", b"x") == Err(SecretError::Denied), "policy: secret_set without secrets refused");
    t.check(secret_get_checked("plattest-policy", "a") == Err(SecretError::Denied) && !secret_available(),
        "policy: secret_get / secret_available without secrets refused");
    t.check(plat::autostart_set(true, &[]) == Err(plat::AutostartError::Denied) && !policy::autostart_allowed(),
        "policy: autostart_set without autostart refused (nothing written)");
    let caps = plat::capabilities();
    t.check(caps.supports(plat::CAP_FILE_READ | plat::CAP_FILE_WRITE), "policy: capabilities keep the files");
    t.check(!caps.supports(plat::CAP_PRINT) && !caps.supports(plat::CAP_NATIVE_PLUGINS), "policy: capabilities drop print and plugins");

    // the network
    let at = |p: u16| SocketAddr::new(IpAddr::LOCALHOST_V4, p);
    let denied = |r: Result<(), NetErrorKind>| r == Err(NetErrorKind::Denied);
    t.check(TcpStream::connect(&at(port), 2000).is_ok(), "policy: connect to a net.connect address");
    let other = if port == u16::MAX { port - 1 } else { port + 1 };
    t.check(denied(TcpStream::connect(&at(other), 2000).map(|_| ()).map_err(|e| e.kind)), "policy: connect to another port is Denied");
    t.check(TcpListener::bind(&at(0)).is_ok(), "policy: listen on a free loopback port (net.listen 127.0.0.1:0)");
    let any = SocketAddr::new(IpAddr::UNSPECIFIED_V4, 0);
    t.check(denied(TcpListener::bind(&any).map(|_| ()).map_err(|e| e.kind)), "policy: listen on every interface is Denied");
    t.check(denied(TcpListener::bind(&at(other)).map(|_| ()).map_err(|e| e.kind)), "policy: listen on a fixed port is Denied");
    let udp = UdpSocket::bind(&at(0));
    t.check(udp.is_ok(), "policy: a UDP client socket (port 0)");
    t.check(denied(UdpSocket::bind(&at(other)).map(|_| ()).map_err(|e| e.kind)), "policy: a UDP socket on a fixed port is Denied");
    if let Ok(u) = &udp {
        t.check(u.send_to(b"x", &at(port)).is_ok(), "policy: a datagram to a net.connect address");
        t.check(denied(u.send_to(b"x", &at(9)).map(|_| ()).map_err(|e| e.kind)), "policy: a datagram elsewhere is Denied");
    }
    t.check(policy::net_name_allowed("a.example.test") && policy::net_name_allowed("b.a.example.test.") && policy::net_name_allowed("PRINTER.lan"),
        "policy: names of the rules");
    t.check(!policy::net_name_allowed("example.test") && !policy::net_name_allowed("evil.org") && !policy::net_name_allowed("xexample.test"),
        "policy: other names refused (*.name: only names below it)");
    let far = |p: u16| SocketAddr::new(IpAddr::V4([10, 1, 2, 3]), p);
    t.check(!policy::net_connect_allowed(&far(443)), "policy: an address before its name was resolved");
    policy::net_resolved("www.example.test", &[far(443)]);
    t.check(policy::net_connect_allowed(&far(443)) && !policy::net_connect_allowed(&far(80)), "policy: a resolved address with the name's ports");
    let evil = SocketAddr::new(IpAddr::V4([10, 9, 9, 9]), 443);
    policy::net_resolved("evil.org", &[evil]);
    t.check(!policy::net_connect_allowed(&evil), "policy: a refused name registers nothing");
    policy::net_resolved("printer.lan", &[SocketAddr::new(IpAddr::V4([192, 168, 7, 7]), 631)]);
    t.check(policy::net_connect_allowed(&SocketAddr::new(IpAddr::V4([192, 168, 7, 7]), 632)), "policy: a port range of a name");
    t.check(policy::net_connect_allowed(&SocketAddr::new(IpAddr::V4([8, 8, 8, 8]), 53)), "policy: DNS (port 53) with name rules");
    if cfg!(windows) {
        t.check(denied(plat::net::resolve("evil.org", 80).map(|_| ()).map_err(|e| e.kind)), "policy: resolve of a refused name is Denied");
    }
    t.check(denied(plat::net::connect_unix(&p("no/sock"), 500).map(|_| ()).map_err(|e| e.kind)), "policy: a Unix socket outside fs.write is Denied");
    drop(listener);

    // the audit
    let d = policy::denials();
    let has = |kind: &str, end: &str| d.iter().any(|x| x.kind == kind && x.target.ends_with(end));
    t.check(has("fs.read", "secret.txt") && has("fs.write", "b.txt") && has("process", "plattest-other-program")
        && has("url", "ftp://example.org") && has("plugins", "plugin.dll") && has("print", "") && has("secrets", "plattest-policy") && has("autostart", "")
        && has("net.connect", &format!(":{other}")) && has("net.listen", "0.0.0.0:0") && has("net.connect", "evil.org"),
        "policy: denials recorded");
    t.info(&format!("  {} denial(s) recorded\n", d.len()));

    key.trusted(|| rm_tree(&base));
    t.check(key.trusted(|| plat::file_exists(&base)) == 0, "policy: scratch tree removed");
}
