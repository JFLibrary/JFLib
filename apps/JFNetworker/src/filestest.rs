// filestest -- checks of the files on devices, TFTP and FTP (group files of --selftest:
// jfn_model::files, jfn_sim::files, tftpd, tftpc, ftpd, ftpc, xfertrace, cli/files.rs): the store
// (paths of both kinds, directories, the system image, limits, records in the project file), the
// command lines (dir, show flash:, more, delete, copy; ls, cat, echo > FILE, cp, mkdir, rm,
// rmdir, touch), a configuration backup to TFTP and the restore on the nanosecond, a transfer
// that loses an ACK, a missing file, a read-only server, a quota, block size options, FTP with
// login, listing, upload and download in passive and active mode, a wrong password, a full
// quota, the problems list, the project round trip and determinism
//
// The LAN (device ids in order): R1 (JF-R4, Gi0/0/0 10.0.0.1/24), SW1, SRV1 (10.0.0.10, a
// TFTP server on /srv/tftp that takes writes, an FTP server with anonymous login and the user
// anna in /srv/ftp/anna), PC1 (10.0.0.20).  1 Gbit/s copper with 50 ns of cable: a frame of n
// bytes takes (max(n, 60) + 24) * 8 + 50 ns per cable and twice that over SW1.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::{Json, Value};
use jfn_model::files::{File, FsErr, Store};
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::{Action, Engine, Hop, Note, Report, XNote, SEC};
use netpkt::decode::{self, Proto, P_TFTP};
use netpkt::tftp::Packet;
use netpkt::udp::UdpView;

use crate::selftest::T;

const BUDGET: usize = 4_000_000;
const R1: u32 = 1;
const SW1: u32 = 2;
const SRV1: u32 = 3;
const PC1: u32 = 4;

fn s(x: &str) -> String {
    String::from(x)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

fn text_file(path: &str, text: &str) -> File {
    File { path: s(path), data: Vec::from(text.as_bytes()), mtime: 1_767_225_600, dir: false }
}

fn dir(path: &str) -> File {
    File { path: s(path), data: Vec::new(), mtime: 1_767_225_600, dir: true }
}

/// 3000 bytes the transfers carry (i * 7 mod 251)
fn big() -> Vec<u8> {
    (0..3000u32).map(|i| (i * 7 % 251) as u8).collect()
}

/// the LAN of the header comment; `tftp` and `ftp` are the servers' settings
fn project(tftp: &str, ftp: &str) -> Project {
    let mut p = Project::new("files", 11);
    for (i, m) in ["jf-r4", "jf-s24p", "server-1u", "pc"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 120, 0);
    }
    let pr = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let _ = p.connect(&pr(R1, "gi0/0/0"), &pr(SW1, "gi1/0/1"), None);
    let _ = p.connect(&pr(SRV1, "eth0"), &pr(SW1, "gi1/0/2"), None);
    let _ = p.connect(&pr(PC1, "eth0"), &pr(SW1, "gi1/0/3"), None);
    let cfg = [
        (R1, "R1", "hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\nend\n"),
        (SW1, "SW1", "hostname SW1\n!\nend\n"),
        (SRV1, "SRV1", "hostname SRV1\nip addr add 10.0.0.10/24 dev eth0\n"),
        (PC1, "PC1", "hostname PC1\nip addr add 10.0.0.20/24 dev eth0\n"),
    ];
    for (id, name, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = s(c);
        }
    }
    if let Some(d) = p.device_mut(SRV1) {
        d.services = alloc::vec![def("tftp-server", "tftp", tftp), def("ftp-server", "ftp", ftp)];
        d.files = alloc::vec![
            File { path: s("/srv/tftp/big.bin"), data: big(), mtime: 1_767_225_600, dir: false },
            dir("/srv/ftp/anna"),
            dir("/srv/ftp/pub"),
            text_file("/srv/ftp/pub/readme.txt", "Willkommen.\n"),
        ];
    }
    p
}

const TFTP_RW: &str = r#"{"writable": true}"#;
const FTP_ANNA: &str = r#"{"users": [{"name": "anna", "password": "geheim", "home": "/srv/ftp/anna"}]}"#;

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

/// lines typed at a device, then `d` of virtual time: what it printed
fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    run_for(e, d);
    out(e, dev)
}

/// the TFTP packets on the cables: (sent, arrived, from device, packet)
fn tftp_hops(e: &Engine) -> Vec<(u64, u64, u32, u16, u16, Packet)> {
    e.trace
        .hops()
        .filter(|h| h.mask & P_TFTP != 0 && h.drop.is_none())
        .filter_map(|h: &Hop| {
            let st = decode::stack(&h.bytes);
            let (o, _) = st.find(Proto::Udp)?;
            let u = UdpView::parse(&h.bytes[o..])?;
            Some((h.t_tx, h.t_rx, h.from.0, u.src_port(), u.dst_port(), Packet::parse(u.payload()).ok()?))
        })
        .collect()
}

/// the notes of transfers of a device
fn xnotes(e: &Engine, dev: u32) -> Vec<(u64, XNote)> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Xfer(n) = &x.note { Some((x.t, n.clone())) } else { None }).collect()
}

pub(crate) fn run(t: &mut T) {
    store(t);
    records(t);
    ios_cli(t);
    linux_cli(t);
    backup(t);
    loss(t);
    errors(t);
    options(t);
    ftp(t);
    problems(t);
    determinism(t);
}

// ---------------------------------------------------------------- the store

fn store(t: &mut T) {
    let r4 = jfn_model::catalog::model("jf-r4");
    let pc = jfn_model::catalog::model("pc");
    let (Some(r4), Some(pc)) = (r4, pc) else {
        t.check("models", false);
        return;
    };
    let mut ios = Store::for_model(r4);
    t.eq("router flash", (ios.capacity, ios.image.clone()), (256 << 20, Some((s("jf-r4.bin"), (40 << 20) + (312 << 10)))));
    t.eq("ios paths", [ios.norm("r1.cfg"), ios.norm("flash:r1.cfg"), ios.norm("FLASH:/r1.cfg"), ios.norm("nvram:startup-config")], [Ok(s("flash:/r1.cfg")), Ok(s("flash:/r1.cfg")), Ok(s("flash:/r1.cfg")), Ok(s("nvram:/startup-config"))]);
    t.eq("ios bad names", [ios.norm("flash:a/b"), ios.norm("disk0:x"), ios.norm("flash:a:b")], [Err(FsErr::BadName), Err(FsErr::NotFound), Err(FsErr::BadName)]);
    t.check("ios write", ios.write("r1.cfg", b"hostname R1\n", 100).is_ok());
    let names: Vec<String> = ios.list("flash:").unwrap_or_default().into_iter().map(|(n, _)| n).collect();
    t.eq("image first", names, alloc::vec![s("jf-r4.bin"), s("r1.cfg")]);
    t.eq("image is read-only", (ios.read("jf-r4.bin").err(), ios.remove("flash:jf-r4.bin").err(), ios.write("jf-r4.bin", b"x", 0).err()), (Some(FsErr::ReadOnly), Some(FsErr::ReadOnly), Some(FsErr::ReadOnly)));
    t.eq("used with the image", ios.used(), (40 << 20) + (312 << 10) + 12);
    t.eq("no directories on ios", ios.mkdir("flash:x", 0, false), Err(FsErr::ReadOnly));
    let mut lx = Store::for_model(pc);
    t.eq("pc disk", (lx.capacity, lx.image.clone()), (512 << 30, None));
    t.eq("linux paths", [lx.norm("a.txt"), lx.norm("/srv//tftp/./x"), lx.norm("/srv/tftp/../ftp/y"), lx.norm("../../etc")], [Ok(s("/root/a.txt")), Ok(s("/srv/tftp/x")), Ok(s("/srv/ftp/y")), Ok(s("/etc"))]);
    t.check("builtin dirs", lx.is_dir("/srv/tftp") && lx.is_dir("/home/user") && lx.is_dir("/") && !lx.is_dir("/opt"));
    t.eq("write without its directory", lx.write("/opt/x", b"1", 0), Err(FsErr::NotFound));
    t.eq("mkdir -p", lx.mkdir("/opt/a/b", 5, true), Ok(()));
    t.eq("mkdir again", (lx.mkdir("/opt/a", 5, false), lx.mkdir("/opt/a", 5, true)), (Err(FsErr::Exists), Ok(())));
    t.check("write into it", lx.write("/opt/a/b/c.txt", b"abc", 7).is_ok() && lx.read("/opt/a/b/c.txt") == Ok(&b"abc"[..]));
    t.eq("append", (lx.append("/opt/a/b/c.txt", b"def", 8), lx.read("/opt/a/b/c.txt").map(Vec::from)), (Ok(()), Ok(b"abcdef".to_vec())));
    t.eq("rmdir not empty", lx.rmdir("/opt/a"), Err(FsErr::NotEmpty));
    t.eq("builtin cannot go", (lx.rmdir("/srv/tftp"), lx.remove("/tmp")), (Err(FsErr::ReadOnly), Err(FsErr::IsDir)));
    t.eq("a file is no directory", lx.write("/opt/a/b/c.txt/d", b"1", 0), Err(FsErr::NotDir));
    t.eq("under", lx.under("/opt"), 6);
    let recs = lx.records();
    t.eq("records in path order", recs.iter().map(|f| (f.path.clone(), f.dir)).collect::<Vec<_>>(), alloc::vec![(s("/opt"), true), (s("/opt/a"), true), (s("/opt/a/b"), true), (s("/opt/a/b/c.txt"), false)]);
    let mut back = Store::for_model(pc);
    back.load(&recs);
    t.eq("load gives the same", back.records(), recs.clone());
    // a record under a missing directory, a builtin directory and a bad path are left out
    back.load(&[text_file("/nowhere/x", "1"), dir("/srv"), text_file("", "y"), text_file("/tmp/ok", "2")]);
    t.eq("load leaves out", back.records().iter().map(|f| f.path.clone()).collect::<Vec<_>>(), alloc::vec![s("/tmp/ok")]);
    t.eq("remove tree", (lx.remove_tree("/opt"), lx.len()), (Ok(()), 0));
    // the limit: a store of 1000 bytes
    let mut small = Store::new(false, 1000, None);
    t.check("fits", small.write("/tmp/a", &[1; 600], 0).is_ok());
    t.eq("does not fit", small.write("/tmp/b", &[1; 500], 0), Err(FsErr::NoSpace));
    t.check("replacing counts the difference", small.write("/tmp/a", &[2; 1000], 0).is_ok());
    t.eq("free", small.free(), 0);
    t.check("text or not", jfn_model::files::is_text(b"a\tb\r\n") && !jfn_model::files::is_text(&[0, 1, 2]) && !jfn_model::files::is_text(&[0xFF]));
}

fn records(t: &mut T) {
    let f = text_file("flash:/r1-config", "hostname R1\n");
    t.eq("text record", agent_core::json::stringify(&f.to_json()).unwrap_or_default(), s("{\"path\":\"flash:/r1-config\",\"text\":\"hostname R1\\n\",\"mtime\":1767225600}"));
    let b = File { path: s("/srv/tftp/a.bin"), data: alloc::vec![0, 1, 2, 255], mtime: 0, dir: false };
    t.eq("binary record", agent_core::json::stringify(&b.to_json()).unwrap_or_default(), s("{\"path\":\"/srv/tftp/a.bin\",\"base64\":\"AAEC/w==\"}"));
    t.eq("dir record", agent_core::json::stringify(&dir("/srv/ftp/in").to_json()).unwrap_or_default(), s("{\"path\":\"/srv/ftp/in\",\"dir\":true,\"mtime\":1767225600}"));
    for x in [&f, &b] {
        t.eq(&format!("record round trip {}", x.path), File::from_json(&x.to_json()), Some(x.clone()));
    }
    // the project file keeps them; a device without files reads as before
    let p = project(TFTP_RW, FTP_ANNA);
    let text = p.to_text();
    t.check("in the file", text.contains("\"path\": \"/srv/ftp/pub/readme.txt\"") && text.contains("\"base64\": \""));
    match Project::from_text(&text) {
        Ok(q) => t.eq("project round trip", q.device(SRV1).map(|d| d.files.clone()), p.device(SRV1).map(|d| d.files.clone())),
        Err(_) => t.check("project round trip", false),
    }
    let old = "{\"format\": \"jfn\", \"version\": 1, \"devices\": [{\"id\": 1, \"name\": \"PC1\", \"model\": \"pc\"}]}";
    t.eq("an old file", Project::from_text(old).ok().and_then(|q| q.device(1).map(|d| d.files.len())), Some(0));
    // the engine takes them from the project
    let mut e = engine(&p);
    let mut want = p.device(SRV1).map(|d| d.files.clone()).unwrap_or_default();
    want.sort_by(|a, b| a.path.cmp(&b.path));
    t.eq("engine has them, in path order", e.files(SRV1), want);
    t.eq("a router starts empty", e.files(R1), Vec::new());
    t.eq("but flash has its image", e.file_store(R1).map(|s| s.used()), Some((40 << 20) + (312 << 10)));
    // Action::Files replaces them without a report
    let _ = e.take_reports();
    t.check("set", e.apply(Action::Files { dev: PC1, files: alloc::vec![text_file("/root/n.txt", "1")] }).is_ok());
    t.eq("set them", e.files(PC1).len(), 1);
    t.check("no report", !e.take_reports().iter().any(|r| matches!(r, Report::Files { .. })));
    // a change at the command line reports the path and every record
    let _ = e.cli_line(PC1, "echo zwei > /root/m.txt");
    let rep: Vec<(u64, String, usize)> = e.take_reports().into_iter().filter_map(|r| if let Report::Files { at, path, files, .. } = r { Some((at, path, files.len())) } else { None }).collect();
    t.eq("report", rep, alloc::vec![(0, s("/root/m.txt"), 2)]);
}

// ---------------------------------------------------------------- the command lines

fn ios_cli(t: &mut T) {
    let mut e = engine(&project(TFTP_RW, FTP_ANNA));
    let o = cli(&mut e, R1, &["enable", "dir"], 0);
    t.eq("dir", o, s("Directory of flash:/\n\n    1  -rw-    42262528  Jan 1 2026 00:00:00 +00:00      jf-r4.bin\n\n268435456 bytes total (226172928 bytes free)\n"));
    let o = cli(&mut e, R1, &["copy running-config flash:r1.cfg"], 0);
    t.check("copy to flash", o.contains("bytes copied to flash:/r1.cfg"));
    let run = e.running_config(R1);
    t.eq("the file is the running configuration", e.file_store(R1).and_then(|s| s.read("flash:/r1.cfg").ok().map(Vec::from)), Some(run.clone().into_bytes()));
    let o = cli(&mut e, R1, &["show flash:"], 0);
    t.check("show flash:", o.starts_with("-#- --length-- ---------date/time--------- path\n  1   42262528 ") && o.contains(&format!("  2 {:>10} ", run.len())) && o.ends_with(&format!("\n{} bytes available ({} bytes used)\n", (256u64 << 20) - (40 << 20) - (312 << 10) - run.len() as u64, (40u64 << 20) + (312 << 10) + run.len() as u64)));
    let o = cli(&mut e, R1, &["more flash:r1.cfg"], 0);
    t.eq("more", o, run.clone());
    let o = cli(&mut e, R1, &["more flash:nope"], 0);
    t.eq("more of nothing", o, s("%Error opening flash:/nope (No such file or directory)\n"));
    // copying onto a file asks first; "n" keeps it
    let _ = cli(&mut e, R1, &["copy running-config flash:"], 0);
    t.eq("asks for the name", e.cli_prompt(R1), s("Name in flash: [running-config]? "));
    let _ = cli(&mut e, R1, &["r1.cfg"], 0);
    t.eq("asks to replace", e.cli_prompt(R1), s("flash:/r1.cfg exists already. Replace it? [y/n] "));
    let o = cli(&mut e, R1, &["n"], 0);
    t.eq("kept", (o, e.cli_prompt(R1)), (s("Nothing copied.\n"), s("R1#")));
    // delete asks twice; the image cannot go
    let _ = cli(&mut e, R1, &["delete flash:r1.cfg"], 0);
    t.eq("delete asks", e.cli_prompt(R1), s("File to delete [r1.cfg]? "));
    let _ = cli(&mut e, R1, &[""], 0);
    t.eq("delete is sure", e.cli_prompt(R1), s("Delete flash:/r1.cfg for good? [y/n] "));
    let _ = cli(&mut e, R1, &["y"], 0);
    t.check("deleted", e.file_store(R1).is_some_and(|s| !s.exists("flash:/r1.cfg")));
    let o = cli(&mut e, R1, &["delete /force flash:jf-r4.bin"], 0);
    t.eq("image", o, s("%Error deleting flash:/jf-r4.bin (Permission denied)\n"));
    let o = cli(&mut e, R1, &["copy flash:jf-r4.bin tftp://10.0.0.10/img"], 0);
    t.eq("image not copied", o, s("%Error: flash:/jf-r4.bin is the system image; the simulator does not copy it\n"));
    // startup-config in nvram:
    let o = cli(&mut e, R1, &["dir nvram:"], 0);
    let start = e.startup_config(R1);
    t.eq("nvram", o, format!("Directory of nvram:/\n\n    1  -rw- {:>11}  {:<30}  startup-config\n\n524288 bytes total ({} bytes free)\n", start.len(), "<no date>", 524288 - start.len()));
    let o = cli(&mut e, R1, &["copy nvram:startup-config flash:start.cfg"], 0);
    t.check("startup to flash", o.contains("copied to flash:/start.cfg"));
    let o = cli(&mut e, R1, &["copy flash:start.cfg startup-config", "startup-config"], 0);
    t.check("flash to startup", o.contains("Saved as the startup configuration."));
    // the FTP login of the client is configuration
    let _ = cli(&mut e, R1, &["configure terminal", "ip ftp username anna", "ip ftp password geheim", "end"], 0);
    t.check("ip ftp in the configuration", e.running_config(R1).contains("ip ftp username anna\nip ftp password geheim\n!\n"));
    let _ = cli(&mut e, R1, &["configure terminal", "no ip ftp password", "end"], 0);
    t.check("no ip ftp password", !e.running_config(R1).contains("ip ftp password"));
    let o = cli(&mut e, R1, &["copy run f"], 0);
    t.check("ambiguous", o.contains("% Ambiguous command"));
    let h = e.cli_help(R1, "copy running-config ");
    t.check("help of copy", h.contains("tftp:") && h.contains("flash:") && h.contains("ftp:") && h.contains("startup-config") && h.contains("WORD"));
}

fn linux_cli(t: &mut T) {
    let mut e = engine(&project(TFTP_RW, FTP_ANNA));
    let o = cli(&mut e, PC1, &["ls /"], 0);
    t.eq("ls /", o, s("etc  home  root  srv  tmp  var\n"));
    let o = cli(&mut e, PC1, &["ls /etc"], 0);
    t.eq("ls /etc", o, s("hostname  hosts  resolv.conf\n"));
    let o = cli(&mut e, PC1, &["echo hallo welt > notiz.txt", "echo zweite >> notiz.txt", "cat notiz.txt"], 0);
    t.eq("echo and cat", o, s("hallo welt\nzweite\n"));
    let o = cli(&mut e, PC1, &["ls -l"], 0);
    t.eq("ls -l", o, s("total 4\n-rw-r--r-- 1 root root    18 Jan  1 00:00 notiz.txt\n"));
    let o = cli(&mut e, PC1, &["mkdir /tmp/a/b"], 0);
    t.eq("mkdir without -p", o, s("mkdir: cannot create directory '/tmp/a/b': No such file or directory\n"));
    let o = cli(&mut e, PC1, &["mkdir -p /tmp/a/b", "cp notiz.txt /tmp/a/b", "ls /tmp/a/b", "cat /tmp/a/b/notiz.txt"], 0);
    t.eq("cp into a directory", o, s("notiz.txt\nhallo welt\nzweite\n"));
    let o = cli(&mut e, PC1, &["cp /etc/hostname /tmp/h", "cat /tmp/h"], 0);
    t.eq("cp of a made file", o, s("PC1\n"));
    let o = cli(&mut e, PC1, &["cat /tmp/a", "rm /tmp/a", "rmdir /tmp/a", "rm -r /tmp/a", "ls /tmp"], 0);
    t.eq("errors and rm -r", o, s("cat: /tmp/a: Is a directory\nrm: cannot remove '/tmp/a': Is a directory\nrmdir: failed to remove '/tmp/a': Directory not empty\nh\n"));
    let o = cli(&mut e, PC1, &["rm nope", "rm -f nope", "cat nope", "touch leer", "cat leer", "ls"], 0);
    t.eq("missing files", o, s("rm: cannot remove 'nope': No such file or directory\ncat: nope: No such file or directory\nleer  notiz.txt\n"));
    let o = cli(&mut e, PC1, &["echo x > /nowhere/y", "rm -r /srv"], 0);
    t.eq("shell refusals", o, s("jfn-sh: /nowhere/y: No such file or directory\nrm: cannot remove '/srv': Permission denied\n"));
    let o = cli(&mut e, PC1, &["tftp"], 0);
    t.check("tftp usage", o.starts_with("Usage: tftp [-g|-p]"));
    let o = cli(&mut e, PC1, &["tftp -g -r x -l /nowhere/x 10.0.0.10"], 0);
    t.eq("tftp checks the local file first", o, s("tftp: can't open '/nowhere/x': No such file or directory\n"));
    let (line, choices) = e.cli_complete(PC1, "t");
    t.eq("tab", (line, choices), (None, alloc::vec![s("telnet"), s("tftp"), s("top"), s("touch"), s("traceroute")]));
    t.eq("files of the project", e.files(PC1).iter().map(|f| f.path.clone()).collect::<Vec<_>>(), alloc::vec![s("/root/leer"), s("/root/notiz.txt"), s("/tmp/h")]);
}

// ---------------------------------------------------------------- TFTP

fn backup(t: &mut T) {
    let mut e = engine(&project(TFTP_RW, FTP_ANNA));
    run_for(&mut e, SEC);
    let run = e.running_config(R1);
    let _ = cli(&mut e, R1, &["enable", "copy running-config tftp:"], 0);
    t.eq("asks for the host", e.cli_prompt(R1), s("Host of the server []? "));
    let _ = cli(&mut e, R1, &["10.0.0.10"], 0);
    t.eq("asks for the file", e.cli_prompt(R1), s("File on the server [r1-config]? "));
    let o = cli(&mut e, R1, &[""], 10 * SEC);
    t.eq("the backup's lines", o, format!("Sending {} bytes to tftp://10.0.0.10/r1-config ...\n.##\nDone: {} bytes in 1000 ms ({} bytes/s)\n", run.len(), run.len(), run.len()));
    t.eq("on the server", e.file_store(SRV1).and_then(|s| s.read("/srv/tftp/r1-config").ok().map(Vec::from)), Some(run.clone().into_bytes()));
    // the packets: the first WRQ dies at R1 at 1 s (IOS asks ARP and drops it), the second goes
    // at 2 s and gets ACK 0 from a new port of the server at once (60-byte frames: 722 ns a
    // cable), then DATA 1 (512 bytes, 558-byte frame: 4706 ns), whose ACK waits for the disk
    // (100 us latency, 20 us an access), DATA 2 (the 17 bytes left) and its ACK
    let h = tftp_hops(&e);
    let kinds: Vec<(u32, u16)> = h.iter().filter(|x| x.2 == R1 || x.2 == SRV1).map(|x| (x.2, x.5.opcode())).collect();
    t.eq("packets", kinds, alloc::vec![(R1, 2), (SRV1, 4), (R1, 3), (SRV1, 4), (R1, 3), (SRV1, 4)]);
    let times: Vec<(u64, u64)> = h.iter().filter(|x| x.2 == R1 || x.2 == SRV1).map(|x| (x.0, x.1)).collect();
    t.eq("their times", times, alloc::vec![(2_000_000_000, 2_000_000_722), (2_000_001_444, 2_000_002_166), (2_000_002_888, 2_000_007_594), (2_000_132_300, 2_000_133_022), (2_000_133_744, 2_000_134_490), (2_000_255_236, 2_000_255_958)]);
    let ports: Vec<(u16, u16)> = h.iter().filter(|x| x.2 == R1 || x.2 == SRV1).map(|x| (x.3, x.4)).collect();
    t.eq("the server's transfer port", ports.get(1).map(|p| p.0 != 69 && p.0 > 1023), Some(true));
    t.check("the same ports after", ports[1..].iter().all(|p| (p.0 == ports[1].0 && p.1 == ports[1].1) || (p.0 == ports[1].1 && p.1 == ports[1].0)));
    t.eq("server numbers", (e.metric(SRV1, "tftp", "writes"), e.metric(SRV1, "tftp", "done"), e.metric(SRV1, "tftp", "bytes_received")), (Some(1), Some(1), Some(run.len() as i64)));
    let notes: Vec<XNote> = xnotes(&e, SRV1).into_iter().map(|x| x.1).collect();
    t.check("notes", notes.iter().any(|n| matches!(n, XNote::TftpRequest { write: true, .. })) && notes.iter().any(|n| matches!(n, XNote::TftpDone { write: true, .. })));
    let logs: Vec<String> = e.log_lines(SRV1).into_iter().filter(|l| l.tag == "tftpd").map(|l| l.text).collect();
    t.eq("log", logs, alloc::vec![s("WRQ from 10.0.0.1 for r1-config (512-byte blocks)"), format!("received r1-config from 10.0.0.1: {} bytes", run.len())]);
    // the restore: another name, then the backup into running-config (a merge)
    let _ = cli(&mut e, R1, &["configure terminal", "hostname Kaputt", "end"], 0);
    let o = cli(&mut e, R1, &["copy tftp://10.0.0.10/r1-config running-config"], 0);
    t.eq("asks where to", (o, e.cli_prompt(R1)), (String::new(), s("Copy into [running-config]? ")));
    let o = cli(&mut e, R1, &[""], 5 * SEC);
    t.check("restored", o.starts_with("Fetching tftp://10.0.0.10/r1-config ...\n##\nDone: ") && o.contains("%SYS-5-CONFIG_I: configuration changed from tftp://10.0.0.10/r1-config"));
    t.eq("the name is back", e.node(R1).map(|n| n.name.clone()), Some(s("R1")));
    t.eq("the running configuration", e.running_config(R1), run.clone());
    // into the startup configuration
    let _ = e.take_reports();
    let _ = cli(&mut e, R1, &["copy tftp: startup-config", "", "r1-config", ""], 5 * SEC);
    t.eq("startup", e.startup_config(R1), run.clone());
    // from flash: to the server and back into flash:
    let _ = cli(&mut e, R1, &["copy running-config flash:a.cfg"], 0);
    let o = cli(&mut e, R1, &["copy flash:a.cfg tftp:", "", ""], 5 * SEC);
    t.check("flash to tftp", o.contains("Sending") && o.contains("Done:"));
    let o = cli(&mut e, R1, &["copy tftp://10.0.0.10/a.cfg flash:", "b.cfg"], 5 * SEC);
    t.check("tftp to flash", o.contains("Done:") && e.file_store(R1).is_some_and(|s| s.read("flash:/b.cfg") == Ok(run.as_bytes())));
}

fn loss(t: &mut T) {
    let p = project(TFTP_RW, FTP_ANNA);
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &["tftp -g -r big.bin 10.0.0.10"], 0);
    // run until DATA 1 is on SRV1's cable, then lose what comes back to SRV1 for 100 ms
    let mut tries = 0;
    while !tftp_hops(&e).iter().any(|x| x.2 == SRV1 && matches!(x.5, Packet::Data { block: 1, .. })) && tries < 10_000 {
        run_for(&mut e, 1000);
        tries += 1;
    }
    let Some(link) = p.link_at(SRV1, "eth0").map(|l| l.id) else {
        t.check("link of SRV1", false);
        return;
    };
    let lossy = |e: &mut Engine, ppm: u32| e.apply(Action::Impair { link, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(ppm) });
    let _ = lossy(&mut e, 1_000_000);
    run_for(&mut e, 100_000_000);
    let _ = lossy(&mut e, 0);
    run_for(&mut e, 5 * SEC);
    let o = out(&mut e, PC1);
    t.eq("quiet on success", o, String::new());
    t.eq("the file", e.file_store(PC1).and_then(|s| s.read("big.bin").ok().map(Vec::from)), Some(big()));
    t.eq("one retransmission", e.metric(SRV1, "tftp", "retransmits"), Some(1));
    let again: Vec<(u64, XNote)> = xnotes(&e, SRV1).into_iter().filter(|x| matches!(x.1, XNote::TftpAgain { .. })).collect();
    // DATA 1 left SRV1 at 1.000124332 s (the RRQ at 1.000004332 after PC1's ARP, then 120 us of
    // disk); its ACK was lost: a second later it goes again
    t.eq("DATA 1 again after the timeout", again, alloc::vec![(2_000_124_332, XNote::TftpAgain { block: 1, n: 1 })]);
    let dup: Vec<(u64, XNote)> = xnotes(&e, PC1).into_iter().filter(|x| matches!(x.1, XNote::TftpDupData { .. })).collect();
    t.eq("PC1 acknowledges it again", dup.iter().map(|x| x.1.clone()).collect::<Vec<_>>(), alloc::vec![XNote::TftpDupData { block: 1 }]);
    let blocks: Vec<u16> = tftp_hops(&e).iter().filter(|x| x.2 == SRV1).filter_map(|x| if let Packet::Data { block, .. } = x.5 { Some(block) } else { None }).collect();
    t.eq("blocks sent", blocks, alloc::vec![1, 1, 2, 3, 4, 5, 6]);
    // then lock-step, 10856 ns a block (2 x 4706 ns DATA, 2 x 722 ns ACK), the 440 bytes of the
    // last one 8260 ns
    t.eq("done at", xnotes(&e, PC1).into_iter().filter(|x| matches!(x.1, XNote::TftpDone { .. })).map(|x| x.0).collect::<Vec<_>>(), alloc::vec![2_000_186_872]);
}

fn errors(t: &mut T) {
    let mut e = engine(&project(r#"{"writable": false}"#, FTP_ANNA));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["tftp -g -r nope 10.0.0.10"], 3 * SEC);
    t.eq("missing file", o, s("tftp: server error: (1) File not found\n"));
    let o = cli(&mut e, PC1, &["echo 1 > x", "tftp -p -l x 10.0.0.10"], 3 * SEC);
    t.eq("read-only server", o, s("tftp: server error: (2) Access violation\n"));
    let o = cli(&mut e, PC1, &["tftp -g -r ../../etc/hostname 10.0.0.10"], 3 * SEC);
    t.eq("outside the root", o, s("tftp: server error: (2) Access violation\n"));
    t.eq("errors counted", e.metric(SRV1, "tftp", "errors"), Some(3));
    let refused: Vec<u16> = xnotes(&e, SRV1).into_iter().filter_map(|x| if let XNote::TftpRefused { code, .. } = x.1 { Some(code) } else { None }).collect();
    t.eq("refusal notes", refused, alloc::vec![1, 2, 2]);
    // the IOS-like side words it so
    let o = cli(&mut e, R1, &["enable", "copy tftp://10.0.0.10/nope flash:", "nope"], 5 * SEC);
    t.check("ios missing", o.ends_with("\n%Error opening tftp://10.0.0.10/nope (File not found)\n"));
    // a quota of 4 KiB with big.bin's 3000 bytes in the root: another 3000 do not fit (error 3
    // at the third block of 512)
    let mut e = engine(&project(r#"{"writable": true, "quota_kb": 4}"#, FTP_ANNA));
    run_for(&mut e, SEC);
    let mut p = Vec::new();
    p.extend_from_slice(&big());
    let _ = e.apply(Action::Files { dev: PC1, files: alloc::vec![File { path: s("/root/big.bin"), data: big(), mtime: 0, dir: false }] });
    let o = cli(&mut e, PC1, &["tftp -p -l big.bin -r big2.bin 10.0.0.10"], 5 * SEC);
    t.eq("quota", o, s("tftp: server error: (3) Disk full or allocation exceeded\n"));
    t.check("nothing stored", e.file_store(SRV1).is_some_and(|s| !s.exists("/srv/tftp/big2.bin")));
    // tsize tells it before a byte goes
    let o = cli(&mut e, PC1, &["tftp -p -b 512 -l big.bin -r big3.bin 10.0.0.10"], 5 * SEC);
    t.eq("quota with tsize", o, s("tftp: server error: (3) Disk full or allocation exceeded\n"));
    let data: usize = tftp_hops(&e).iter().filter(|x| x.2 == PC1).filter_map(|x| if let Packet::Data { data, .. } = &x.5 { Some(data.len()) } else { None }).sum();
    t.eq("no data after the refused tsize", data, 3 * 512);
}

fn options(t: &mut T) {
    let mut e = engine(&project(TFTP_RW, FTP_ANNA));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["tftp -g -b 1428 -r big.bin -l b.bin 10.0.0.10"], 5 * SEC);
    t.eq("quiet", o, String::new());
    let h = tftp_hops(&e);
    let oack = h.iter().find_map(|x| if let Packet::Oack { opts } = &x.5 { Some(opts.clone()) } else { None });
    t.eq("OACK", oack, Some(alloc::vec![(s("blksize"), s("1428")), (s("tsize"), s("3000"))]));
    let sizes: Vec<usize> = h.iter().filter(|x| x.2 == SRV1).filter_map(|x| if let Packet::Data { data, .. } = &x.5 { Some(data.len()) } else { None }).collect();
    t.eq("blocks of 1428", sizes, alloc::vec![1428, 1428, 144]);
    t.eq("ACK 0 after the OACK", h.iter().filter(|x| x.2 == PC1).map(|x| x.5.opcode()).collect::<Vec<_>>(), alloc::vec![1, 4, 4, 4, 4]);
    t.eq("the file", e.file_store(PC1).and_then(|s| s.read("b.bin").ok().map(Vec::from)), Some(big()));
    // a server without options answers as RFC 1350
    let mut e = engine(&project(r#"{"options": false}"#, FTP_ANNA));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &["tftp -g -b 1428 -r big.bin 10.0.0.10"], 5 * SEC);
    let sizes: Vec<usize> = tftp_hops(&e).iter().filter(|x| x.2 == SRV1).filter_map(|x| if let Packet::Data { data, .. } = &x.5 { Some(data.len()) } else { None }).collect();
    t.eq("blocks of 512", sizes, alloc::vec![512, 512, 512, 512, 512, 440]);
    // a frame of the transfer in the dissector
    let f = e.trace.hops().find(|h| h.mask & P_TFTP != 0 && h.from.0 == SRV1).map(|h| h.bytes.to_vec()).unwrap_or_default();
    t.eq("dissector", decode::info(&f), (Proto::Tftp, s("Data Packet, Block: 1")));
}

// ---------------------------------------------------------------- FTP

fn ftp(t: &mut T) {
    let mut e = engine(&project(TFTP_RW, FTP_ANNA));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["echo Messwerte > data.txt", "curl -v -T data.txt ftp://anna:geheim@10.0.0.10/"], 5 * SEC);
    let cmds: Vec<&str> = o.lines().filter(|l| l.starts_with("> ") || l.starts_with("< ")).collect();
    t.eq(
        "upload dialogue",
        cmds,
        alloc::vec![
            "< 220 JFNetworker FTP server ready.",
            "> USER anna",
            "< 331 Password, please.",
            "> PASS geheim",
            "< 230 Logged in.",
            "> PWD",
            "< 257 \"/\" is the current directory.",
            "> EPSV",
            "< 229 Extended passive mode on (|||54018|).",
            "> TYPE I",
            "< 200 Binary mode (type I).",
            "> STOR data.txt",
            "< 150 Ready for data.txt.",
            "< 226 File stored.",
            "> QUIT",
            "< 221 Goodbye.",
        ]
    );
    t.eq("uploaded", e.file_store(SRV1).and_then(|s| s.read("/srv/ftp/anna/data.txt").ok().map(Vec::from)), Some(b"Messwerte\n".to_vec()));
    let o = cli(&mut e, PC1, &["curl -u anna:geheim ftp://10.0.0.10/"], 5 * SEC);
    t.eq("listing", o, s("-rw-r--r--    1 ftp      ftp            10 Jan 01 00:00 data.txt\n"));
    let o = cli(&mut e, PC1, &["curl -u anna:geheim -l ftp://10.0.0.10/"], 5 * SEC);
    t.eq("names", o, s("data.txt\n"));
    let o = cli(&mut e, PC1, &["curl -u anna:geheim ftp://10.0.0.10/data.txt -o copy.txt"], 5 * SEC);
    t.eq("download into a file", o, String::new());
    let o = cli(&mut e, PC1, &["cat copy.txt"], 0);
    t.eq("download", o, s("Messwerte\n"));
    let o = cli(&mut e, PC1, &["curl ftp://10.0.0.10/pub/readme.txt"], 5 * SEC);
    t.eq("anonymous with CWD", o, s("Willkommen.\n"));
    let o = cli(&mut e, PC1, &["curl -T data.txt ftp://10.0.0.10/pub/"], 5 * SEC);
    t.eq("anonymous may not write", o, s("curl: (25) Failed FTP upload: 550\n"));
    let o = cli(&mut e, PC1, &["curl ftp://10.0.0.10/nope.txt"], 5 * SEC);
    t.eq("missing file", o, s("curl: (78) RETR response: 550\n"));
    let o = cli(&mut e, PC1, &["curl ftp://10.0.0.10/nodir/x"], 5 * SEC);
    t.eq("missing directory", o, s("curl: (9) Server denied you to change to the given directory\n"));
    // a wrong password: 530 a second later
    let _ = e.take_reports();
    let t0 = e.now();
    let _ = e.cli_line(PC1, "curl -v -u anna:falsch ftp://10.0.0.10/");
    run_for(&mut e, 5 * SEC);
    let o = out(&mut e, PC1);
    t.check("refused", o.ends_with("< 530 Wrong user name or password.\r\ncurl: (67) Access denied: 530\n"));
    let failed: Vec<u64> = xnotes(&e, SRV1).into_iter().filter(|x| matches!(x.1, XNote::FtpLoginFailed { .. }) && x.0 >= t0).map(|x| x.0).collect();
    t.eq("failed login noted", failed.len(), 1);
    t.eq("numbers", (e.metric(SRV1, "ftp", "logins"), e.metric(SRV1, "ftp", "failed_logins"), e.metric(SRV1, "ftp", "uploads"), e.metric(SRV1, "ftp", "downloads"), e.metric(SRV1, "ftp", "listings")), (Some(8), Some(1), Some(1), Some(2), Some(2)));
    // active mode: EPRT, the server connects from port 20
    let o = cli(&mut e, PC1, &["curl -v -P - -u anna:geheim ftp://10.0.0.10/data.txt"], 5 * SEC);
    t.check("active", o.contains("> EPRT |1|10.0.0.20|") && o.contains("< 200 Active mode on.") && o.ends_with("Messwerte\n* Connection #0 to host 10.0.0.10 left intact\n"));
    t.check("from port 20", e.trace.hops().any(|h| h.from.0 == SRV1 && decode::info(&h.bytes).0 == Proto::FtpData));
    // the IOS-like client
    let _ = cli(&mut e, R1, &["enable", "configure terminal", "ip ftp username anna", "ip ftp password geheim", "end"], 0);
    let o = cli(&mut e, R1, &["copy running-config ftp://10.0.0.10/r1.cfg"], 5 * SEC);
    t.check("ios upload", o.starts_with("Sending ") && o.contains("Done: "));
    let run = e.running_config(R1);
    t.eq("ios upload stored", e.file_store(SRV1).and_then(|s| s.read("/srv/ftp/anna/r1.cfg").ok().map(Vec::from)), Some(run.into_bytes()));
    let o = cli(&mut e, R1, &["copy ftp://10.0.0.10/r1.cfg flash:", "r1.cfg"], 5 * SEC);
    t.check("ios download", o.contains("Done: ") && e.file_store(R1).is_some_and(|s| s.exists("flash:/r1.cfg")));
    // a quota of 1 KiB: 552
    let mut e = engine(&project(TFTP_RW, r#"{"quota_kb": 1, "users": [{"name": "anna", "password": "geheim", "home": "/srv/ftp/anna"}]}"#));
    run_for(&mut e, SEC);
    let _ = e.apply(Action::Files { dev: PC1, files: alloc::vec![File { path: s("/root/big.bin"), data: big(), mtime: 0, dir: false }] });
    let o = cli(&mut e, PC1, &["curl -T big.bin ftp://anna:geheim@10.0.0.10/"], 5 * SEC);
    t.eq("quota", o, s("curl: (25) Failed FTP upload: 552\n"));
    t.check("no file", e.file_store(SRV1).is_some_and(|s| !s.exists("/srv/ftp/anna/big.bin")));
    t.check("full noted", xnotes(&e, SRV1).iter().any(|x| matches!(x.1, XNote::FtpFull { .. })));
}

fn problems(t: &mut T) {
    let mut e = engine(&project(r#"{"root": "/srv/nichts"}"#, r#"{"users": [{"name": "bob", "home": "/home/bob"}]}"#));
    run_for(&mut e, 1000);
    let p: Vec<String> = e.problems().into_iter().filter(|p| p.kind == jfn_sim::ProblemKind::FilesDirMissing).map(|p| p.text).collect();
    t.eq("missing directories", p, alloc::vec![s("Dienst ftp auf SRV1: das Verzeichnis /home/bob gibt es nicht."), s("Dienst tftp auf SRV1: das Verzeichnis /srv/nichts gibt es nicht.")]);
    let ok = engine(&project(TFTP_RW, FTP_ANNA));
    t.check("none without", !ok.problems().iter().any(|p| p.kind == jfn_sim::ProblemKind::FilesDirMissing));
    let st: Vec<String> = e.services(SRV1).into_iter().map(|s| s.status).collect();
    t.eq("status lines", st.first().cloned(), Some(s("/srv/nichts: das Verzeichnis fehlt")));
}

fn determinism(t: &mut T) {
    let go = || {
        let mut e = engine(&project(TFTP_RW, FTP_ANNA));
        let _ = cli(&mut e, R1, &["enable", "copy running-config tftp://10.0.0.10/r1-config"], 5 * SEC);
        let _ = cli(&mut e, PC1, &["curl -u anna:geheim ftp://10.0.0.10/"], 5 * SEC);
        let hops: Vec<(u64, u64, Vec<u8>)> = e.trace.hops().map(|h| (h.t_tx, h.t_rx, h.bytes.to_vec())).collect();
        (hops, e.files(SRV1))
    };
    let (a, b) = (go(), go());
    t.check("twice the same", a == b && !a.0.is_empty());
}
