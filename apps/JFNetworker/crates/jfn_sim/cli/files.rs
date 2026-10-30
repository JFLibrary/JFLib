// files -- the files of a device at the command lines (the store: files.rs, jfn_model::files)
//
//   IOS-like   dir [flash:|nvram:|FILE], show flash:, more FILE, delete [/force] FILE (asks
//              first), copy SRC DST with the questions for what is missing, ip ftp username /
//              password (the FTP client's login; in the running configuration)
//   Linux      ls [-l] [-a] [PATH ...], cat FILE (the files of the store after the ones of /etc),
//              rm [-f] [-r] FILE ..., cp SRC DST, mkdir [-p] DIR ..., rmdir DIR ..., touch FILE,
//              echo ... > FILE / >> FILE, tftp (busybox-like: -g / -p, -l, -r, -b, HOST [PORT])
//
// copy takes running-config, startup-config (nvram:startup-config), flash:NAME (a bare NAME is in
// flash:), tftp://HOST/NAME, ftp://[USER[:PASS]@]HOST/NAME and the bare "flash:", "tftp:" and
// "ftp:", which make it ask for the host and the names (the questions, progress and results are
// our own words; the defaults in brackets are the last host and the source's name, for a
// configuration "<host>-config").  A configuration copied to running-config is merged as if
// typed (it adds and changes what it names, as IOS does; a saved one has no "no shutdown"), one
// copied to startup-config becomes the startup configuration (Report::Startup); a file that
// exists in flash: is replaced only after "y".  Transfers run as console programs (tftpc.rs, ftpc.rs);
// Ctrl+Shift+6 ends them.
//
// Paths of the shell are relative to /root (the prompt's ~).  The messages follow coreutils and
// bash; tftp's usage is our own text.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, Val};
use jfn_model::files::{FsErr, Stat};

use super::ios::K;
use super::{Ask, AskKind, Mode};
use crate::driver::Report;
use crate::kernel::Engine;
use crate::os::EPOCH_UNIX_NS;
use crate::tftpc::{Sink, Style, TftpClient};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/// what nvram: holds at most (the startup configuration)
const NVRAM_BYTES: u64 = 512 * 1024;

/// a place copy reads from or writes to
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loc {
    Running,
    Startup,
    /// a file in flash: (None: ask for its name)
    Flash(Option<String>),
    /// a server: TFTP or FTP, its host and the file (None: ask)
    Server { ftp: bool, user: Option<String>, pass: Option<String>, host: Option<String>, file: Option<String> },
}

/// what the console asks for now
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    SrcHost,
    SrcFile,
    DstHost,
    DstFile,
    Into,
    Replace,
    DeleteName,
    DeleteSure,
}

/// a copy or a delete in the middle of its questions
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    /// copy: from, to; delete: the file in `dst` (Flash)
    pub src: Loc,
    pub dst: Loc,
    pub step: Step,
    pub delete: bool,
}

/// what the command line keeps for files (CliDev::files)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilesCli {
    pub dialog: Option<Dialog>,
    /// the host of the last transfer (the default of the next question)
    pub last_host: String,
    /// "ip ftp username", "ip ftp password"
    pub ftp_user: String,
    pub ftp_pass: String,
}

/// the IOS commands of files
pub(super) fn build_files(t: &mut Tree<K>, m: [ModeId; 3]) {
    let [user, prv, conf] = m;
    for md in [user, prv] {
        t.add(md, "dir", "List the files of a file system", Some(K::Dir), 0);
        t.add(md, "dir flash:", "The flash memory", Some(K::Dir), 0);
        t.add(md, "dir nvram:", "The memory of the startup configuration", Some(K::Dir), 0);
        t.add(md, "dir WORD=file", "A file system or a file", Some(K::Dir), 0);
        t.add(md, "show flash:", "Files in the flash memory and the space left", Some(K::ShowFlash), 0);
        t.add(md, "more", "Show what a file holds", None, 0);
        t.add(md, "more WORD=file", "flash:NAME, nvram:startup-config or system:running-config", Some(K::More), 0);
    }
    t.add(prv, "delete", "Remove a file from flash", None, 0);
    t.add(prv, "delete WORD=file", "The file (flash:NAME)", Some(K::Delete), 0);
    t.add(prv, "delete /force", "Without asking first", None, 0);
    t.add(prv, "delete /force WORD=file", "The file (flash:NAME)", Some(K::Delete), 0);
    let places: [(&str, &str); 6] = [
        ("running-config", "The configuration in effect now"),
        ("startup-config", "The configuration for the next start"),
        ("flash:", "A file in the flash memory (asks for its name)"),
        ("nvram:", "The startup configuration in nvram"),
        ("tftp:", "A file on a TFTP server (asks for host and name)"),
        ("ftp:", "A file on an FTP server (asks for host and name)"),
    ];
    const ANY: &str = "flash:NAME, tftp://HOST/NAME or ftp://[USER:PASS@]HOST/NAME";
    for (s, sh) in places {
        t.add(prv, &format!("copy {}", s), sh, None, 0);
        for (d, dh) in places {
            if s == d || (s == "running-config" && d == "startup-config") {
                continue;
            }
            t.add(prv, &format!("copy {} {}", s, d), dh, Some(K::Copy), 0);
        }
        t.add(prv, &format!("copy {} WORD=dst", s), ANY, Some(K::Copy), 0);
    }
    t.add(prv, "copy WORD=src", ANY, None, 0);
    for (d, dh) in places {
        t.add(prv, &format!("copy WORD=src {}", d), dh, Some(K::Copy), 0);
    }
    t.add(prv, "copy WORD=src WORD=dst", ANY, Some(K::Copy), 0);
    t.add(conf, "ip ftp", "Settings of the FTP client", None, 0);
    t.add(conf, "ip ftp username", "The user name for FTP servers", None, 0);
    t.add(conf, "ip ftp username WORD=user", "The name", Some(K::IpFtp), 0);
    t.add(conf, "ip ftp password", "The password for FTP servers", None, 0);
    t.add(conf, "ip ftp password WORD=pw", "The password", Some(K::IpFtp), 0);
    t.add_no(conf, "ip ftp username", K::IpFtp);
    t.add_no(conf, "ip ftp password", K::IpFtp);
}

/// a place of copy from its word
fn loc_of(w: &str) -> Result<Loc, String> {
    let lw = w.to_ascii_lowercase();
    if lw == "running-config" || lw == "system:running-config" {
        return Ok(Loc::Running);
    }
    if lw == "startup-config" || lw == "nvram:startup-config" || lw == "nvram:/startup-config" || lw == "nvram:" {
        return Ok(Loc::Startup);
    }
    if lw == "flash:" || lw == "flash:/" {
        return Ok(Loc::Flash(None));
    }
    if let Some(n) = lw.strip_prefix("flash:") {
        return Ok(Loc::Flash(Some(String::from(w[6..].trim_start_matches('/')).replace(char::is_whitespace, "")).filter(|_| !n.is_empty())));
    }
    for (scheme, ftp) in [("tftp:", false), ("ftp:", true)] {
        if lw == scheme {
            return Ok(Loc::Server { ftp, user: None, pass: None, host: None, file: None });
        }
        if let Some(rest) = w.get(scheme.len()..).filter(|_| lw.starts_with(scheme)).and_then(|r| r.strip_prefix("//")) {
            let (auth, rest) = match rest.rfind('@') {
                Some(i) if ftp => (Some(&rest[..i]), &rest[i + 1..]),
                _ => (None, rest),
            };
            let (host, file) = match rest.split_once('/') {
                Some((h, f)) => (h, f),
                None => (rest, ""),
            };
            let (user, pass) = match auth.map(|a| a.split_once(':').unwrap_or((a, ""))) {
                Some((u, p)) => (Some(String::from(u)), Some(String::from(p))),
                None => (None, None),
            };
            let some = |s: &str| if s.is_empty() { None } else { Some(String::from(s)) };
            return Ok(Loc::Server { ftp, user, pass, host: some(host), file: some(file) });
        }
    }
    if w.contains(':') {
        return Err(format!("%Error opening {} (No such file or directory)\n", w));
    }
    Ok(Loc::Flash(Some(String::from(w))))
}

/// "Jan 1 2026 00:00:05 +00:00" (IOS-like dir) of a file time (0: the start of the clock)
fn ios_date(t: i64) -> String {
    let c = datetime::Civil::from_unix(if t == 0 { EPOCH_UNIX_NS / 1_000_000_000 } else { t });
    format!("{} {} {} {:02}:{:02}:{:02} +00:00", MONTHS[(c.month.clamp(1, 12) - 1) as usize], c.day, c.year, c.hour, c.minute, c.second)
}

/// "Jan  1 00:05" (ls -l)
fn ls_date(t: i64) -> String {
    let c = datetime::Civil::from_unix(if t == 0 { EPOCH_UNIX_NS / 1_000_000_000 } else { t });
    format!("{} {:>2} {:02}:{:02}", MONTHS[(c.month.clamp(1, 12) - 1) as usize], c.day, c.hour, c.minute)
}

/// the files of /etc the shell makes from the device's state
const ETC: [&str; 3] = ["hostname", "hosts", "resolv.conf"];

impl Engine {
    // ================================================================ IOS-like

    /// the lines of the running configuration ("ip ftp username ...")
    pub(crate) fn ios_files_config(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else { return String::new() };
        let mut s = String::new();
        if !c.files.ftp_user.is_empty() {
            s.push_str(&format!("ip ftp username {}\n", c.files.ftp_user));
        }
        if !c.files.ftp_pass.is_empty() {
            s.push_str(&format!("ip ftp password {}\n", c.files.ftp_pass));
        }
        if !s.is_empty() {
            s.push_str("!\n");
        }
        s
    }

    /// the commands of files
    pub(crate) fn ios_files(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        match p.cmd {
            K::Dir => {
                let what = p.text("file").map(String::from).unwrap_or_else(|| if p.has("nvram:") { String::from("nvram:") } else { String::from("flash:") });
                let text = self.ios_dir(ni, &what);
                if text.starts_with('%') {
                    return self.ios_err(ni, &text);
                }
                self.out(ni, &text);
                Ok(())
            }
            K::ShowFlash => {
                let text = self.ios_show_flash(ni);
                self.out(ni, &text);
                Ok(())
            }
            K::More => {
                let f = String::from(p.text("file").unwrap_or(""));
                match self.ios_more(ni, &f) {
                    Ok(t) => {
                        self.out(ni, &t);
                        Ok(())
                    }
                    Err(e) => self.ios_err(ni, &e),
                }
            }
            K::Delete => {
                let f = String::from(p.text("file").unwrap_or(""));
                let force = p.has("/force");
                let name = match loc_of(&f) {
                    Ok(Loc::Flash(Some(n))) => n,
                    _ => return self.ios_err(ni, &format!("%Error deleting {} (Invalid argument)\n", f)),
                };
                if force {
                    return self.ios_delete(ni, &name);
                }
                let d = Dialog { src: Loc::Flash(Some(name.clone())), dst: Loc::Flash(Some(name)), step: Step::DeleteName, delete: true };
                self.files_ask(ni, d);
                Ok(())
            }
            K::Copy => {
                // the two places as typed: keywords ("tftp:") or words ("flash:r1-config")
                let ends: Vec<String> = p
                    .vals
                    .iter()
                    .skip(1)
                    .filter_map(|(_, v)| match v {
                        Val::Key(s) | Val::Word(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                if ends.len() != 2 {
                    return self.ios_err(ni, "% Incomplete command.\n");
                }
                let src = match loc_of(&ends[0]) {
                    Ok(l) => l,
                    Err(e) => return self.ios_err(ni, &e),
                };
                let dst = match loc_of(&ends[1]) {
                    Ok(l) => l,
                    Err(e) => return self.ios_err(ni, &e),
                };
                let ok = match (&src, &dst) {
                    (Loc::Server { .. }, Loc::Server { .. }) => false,
                    (Loc::Running, Loc::Running) | (Loc::Startup, Loc::Startup) => false,
                    _ => true,
                };
                if !ok {
                    return self.ios_err(ni, "%Error: source and destination do not fit together\n");
                }
                if src == Loc::Running && dst == Loc::Startup {
                    self.ios_save(ni);
                    return Ok(());
                }
                self.files_ask(ni, Dialog { src, dst, step: Step::SrcHost, delete: false });
                Ok(())
            }
            K::IpFtp => {
                let user = p.has("username");
                let v = if p.no { String::new() } else { String::from(p.text(if user { "user" } else { "pw" }).unwrap_or("")) };
                if let Some(c) = self.cdev_mut(ni) {
                    if user {
                        c.files.ftp_user = v;
                    } else {
                        c.files.ftp_pass = v;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// "Directory of flash:/ ..." (a file system, or one file of flash:)
    fn ios_dir(&self, ni: usize, what: &str) -> String {
        let Some(st) = self.store(ni) else { return String::from("%Error opening flash:/ (No such device)\n") };
        let lw = what.to_ascii_lowercase();
        if lw.starts_with("nvram:") {
            let start = self.cdev(ni).map(|c| c.cfg.startup.len() as u64).unwrap_or(0);
            let mut s = String::from("Directory of nvram:/\n\n");
            if start > 0 {
                s.push_str(&format!("    1  -rw- {:>11}  {:<30}  startup-config\n", start, "<no date>"));
            } else {
                s.push_str("No files in directory\n");
            }
            s.push_str(&format!("\n{} bytes total ({} bytes free)\n", NVRAM_BYTES, NVRAM_BYTES.saturating_sub(start)));
            return s;
        }
        if !lw.starts_with("flash:") && lw.contains(':') {
            return format!("%Error opening {} (No such file or directory)\n", what);
        }
        let list = match st.norm(what) {
            Ok(p) if p == "flash:/" => st.list(&p).unwrap_or_default(),
            Ok(p) => match st.stat(&p) {
                Some(s) => alloc::vec![(String::from(st.name_of(&p)), s)],
                None => return format!("%Error opening {} (No such file or directory)\n", what),
            },
            Err(_) => return format!("%Error opening {} (No such file or directory)\n", what),
        };
        let mut s = String::from("Directory of flash:/\n\n");
        if list.is_empty() {
            s.push_str("No files in directory\n");
        }
        for (i, (name, f)) in list.iter().enumerate() {
            s.push_str(&format!("{:>5}  -rw- {:>11}  {:<30}  {}\n", i + 1, f.size, ios_date(f.mtime), name));
        }
        s.push_str(&format!("\n{} bytes total ({} bytes free)\n", st.capacity, st.free()));
        s
    }

    /// "show flash:": the files with their numbers, sizes and times, the space
    fn ios_show_flash(&self, ni: usize) -> String {
        let Some(st) = self.store(ni) else { return String::new() };
        let mut s = String::from("-#- --length-- ---------date/time--------- path\n");
        for (i, (name, f)) in st.list("flash:/").unwrap_or_default().iter().enumerate() {
            s.push_str(&format!("{:>3} {:>10} {:<27} {}\n", i + 1, f.size, ios_date(f.mtime), name));
        }
        s.push_str(&format!("\n{} bytes available ({} bytes used)\n", st.free(), st.used()));
        s
    }

    /// "more FILE": a text file, the startup or the running configuration
    fn ios_more(&self, ni: usize, f: &str) -> Result<String, String> {
        match loc_of(f) {
            Ok(Loc::Running) => Ok(self.ios_config(ni)),
            Ok(Loc::Startup) => Ok(self.cdev(ni).map(|c| c.cfg.startup.clone()).unwrap_or_default()),
            Ok(Loc::Flash(Some(n))) => {
                let st = self.store(ni).ok_or_else(|| format!("%Error opening {} (No such file or directory)\n", f))?;
                match st.read(&n) {
                    Ok(b) if jfn_model::files::is_text(b) => {
                        let mut t = String::from_utf8_lossy(b).into_owned();
                        if !t.is_empty() && !t.ends_with('\n') {
                            t.push('\n');
                        }
                        Ok(t)
                    }
                    Ok(_) => Err(format!("%Error: flash:/{} holds no text\n", n)),
                    Err(e) => Err(format!("%Error opening flash:/{} ({})\n", n, e.ios_text())),
                }
            }
            _ => Err(format!("%Error opening {} (No such file or directory)\n", f)),
        }
    }

    fn ios_delete(&mut self, ni: usize, name: &str) -> Result<(), String> {
        match self.file_del(ni, name, false) {
            Ok(_) => Ok(()),
            Err(e) => self.ios_err(ni, &format!("%Error deleting flash:/{} ({})\n", name, e.ios_text())),
        }
    }

    /// the default name of a configuration on a server: "<host>-config"
    fn config_name(&self, ni: usize) -> String {
        format!("{}-config", self.host(ni).to_ascii_lowercase())
    }

    /// the name a source gives the destination by default
    fn src_name(&self, ni: usize, src: &Loc) -> String {
        match src {
            Loc::Running => String::from("running-config"),
            Loc::Startup => String::from("startup-config"),
            Loc::Flash(Some(n)) => n.clone(),
            Loc::Server { file: Some(f), .. } => String::from(f.rsplit('/').next().unwrap_or(f)),
            _ => self.config_name(ni),
        }
    }

    /// asks the next question of a dialog, or runs it when nothing is missing
    fn files_ask(&mut self, ni: usize, mut d: Dialog) {
        let last = self.cdev(ni).map(|c| c.files.last_host.clone()).unwrap_or_default();
        let prompt = loop {
            match d.step {
                Step::SrcHost => match &d.src {
                    Loc::Server { host: None, .. } => break format!("Host of the server [{}]? ", last),
                    _ => d.step = Step::SrcFile,
                },
                Step::SrcFile => match &d.src {
                    Loc::Server { file: None, .. } => break String::from("File on the server []? "),
                    Loc::Flash(None) => break String::from("File in flash: []? "),
                    _ => d.step = Step::DstHost,
                },
                Step::DstHost => match &d.dst {
                    Loc::Server { host: None, .. } => break format!("Host of the server [{}]? ", last),
                    _ => d.step = Step::DstFile,
                },
                Step::DstFile => match &d.dst {
                    Loc::Server { file: None, .. } => {
                        let def = match d.src {
                            Loc::Running | Loc::Startup => self.config_name(ni),
                            _ => self.src_name(ni, &d.src),
                        };
                        break format!("File on the server [{}]? ", def);
                    }
                    Loc::Flash(None) => break format!("Name in flash: [{}]? ", self.src_name(ni, &d.src)),
                    _ => d.step = Step::Into,
                },
                Step::Into => match &d.dst {
                    Loc::Running => break String::from("Copy into [running-config]? "),
                    Loc::Startup => break String::from("Copy into [startup-config]? "),
                    _ => d.step = Step::Replace,
                },
                Step::Replace => match &d.dst {
                    Loc::Flash(Some(n)) if self.store(ni).is_some_and(|s| s.exists(n)) => break format!("flash:/{} exists already. Replace it? [y/n] ", n),
                    _ => {
                        if let Some(c) = self.cdev_mut(ni) {
                            c.files.dialog = None;
                        }
                        return self.files_run(ni, d);
                    }
                },
                Step::DeleteName => {
                    let n = self.src_name(ni, &d.src);
                    break format!("File to delete [{}]? ", n);
                }
                Step::DeleteSure => {
                    let n = self.src_name(ni, &d.src);
                    break format!("Delete flash:/{} for good? [y/n] ", n);
                }
            }
        };
        if let Some(c) = self.cdev_mut(ni) {
            c.files.dialog = Some(d);
            c.ask = Some(Ask { prompt, kind: AskKind::Files });
        }
    }

    /// the answer to a question of a dialog
    pub(crate) fn files_answer(&mut self, ni: usize, line: &str) {
        let Some(mut d) = self.cdev_mut(ni).and_then(|c| c.files.dialog.take()) else { return };
        let a = line.trim();
        let last = self.cdev(ni).map(|c| c.files.last_host.clone()).unwrap_or_default();
        let yes = a.is_empty() || a.eq_ignore_ascii_case("y") || a.eq_ignore_ascii_case("yes");
        let pick = |a: &str, def: &str| -> Option<String> { if a.is_empty() { if def.is_empty() { None } else { Some(String::from(def)) } } else { Some(String::from(a)) } };
        match d.step {
            Step::SrcHost | Step::DstHost => {
                let Some(h) = pick(a, &last) else {
                    return self.out(ni, "%Error: a host is needed\n");
                };
                let l = if d.step == Step::SrcHost { &mut d.src } else { &mut d.dst };
                if let Loc::Server { host, .. } = l {
                    *host = Some(h);
                }
            }
            Step::SrcFile => {
                if a.is_empty() {
                    return self.out(ni, "%Error: a file name is needed\n");
                }
                match &mut d.src {
                    Loc::Server { file, .. } => *file = Some(String::from(a)),
                    Loc::Flash(n) => *n = Some(String::from(a.trim_start_matches("flash:").trim_start_matches('/'))),
                    _ => {}
                }
            }
            Step::DstFile => {
                let def = match d.src {
                    Loc::Running | Loc::Startup => self.config_name(ni),
                    _ => self.src_name(ni, &d.src),
                };
                let name = pick(a, &def).unwrap_or(def);
                match &mut d.dst {
                    Loc::Server { file, .. } => *file = Some(name),
                    Loc::Flash(n) => *n = Some(String::from(name.trim_start_matches("flash:").trim_start_matches('/'))),
                    _ => {}
                }
            }
            Step::Into => {
                let want = if d.dst == Loc::Running { "running-config" } else { "startup-config" };
                if !a.is_empty() && !a.eq_ignore_ascii_case(want) {
                    return self.out(ni, &format!("%Error opening {} (Invalid argument)\n", a));
                }
            }
            Step::Replace => {
                if !yes {
                    return self.out(ni, "Nothing copied.\n");
                }
                if let Some(c) = self.cdev_mut(ni) {
                    c.files.dialog = None;
                }
                return self.files_run(ni, d);
            }
            Step::DeleteName => {
                if !a.is_empty() {
                    let n = String::from(a.trim_start_matches("flash:").trim_start_matches('/'));
                    d.src = Loc::Flash(Some(n));
                }
                d.step = Step::DeleteSure;
                return self.files_ask(ni, d);
            }
            Step::DeleteSure => {
                if yes && let Loc::Flash(Some(n)) = &d.src {
                    let n = n.clone();
                    let _ = self.ios_delete(ni, &n);
                }
                return;
            }
        }
        d.step = match d.step {
            Step::SrcHost => Step::SrcFile,
            Step::SrcFile => Step::DstHost,
            Step::DstHost => Step::DstFile,
            Step::DstFile => Step::Into,
            Step::Into => Step::Replace,
            s => s,
        };
        self.files_ask(ni, d);
    }

    /// the bytes of a local source
    fn src_bytes(&self, ni: usize, src: &Loc) -> Result<Vec<u8>, String> {
        match src {
            Loc::Running => Ok(self.ios_config(ni).into_bytes()),
            Loc::Startup => {
                let s = self.cdev(ni).map(|c| c.cfg.startup.clone()).unwrap_or_default();
                if s.is_empty() {
                    return Err(String::from("%Error opening nvram:/startup-config (No such file or directory)\n"));
                }
                Ok(s.into_bytes())
            }
            Loc::Flash(Some(n)) => match self.store(ni).map(|s| s.read(n)) {
                Some(Ok(b)) => Ok(Vec::from(b)),
                Some(Err(FsErr::ReadOnly)) => Err(format!("%Error: flash:/{} is the system image; the simulator does not copy it\n", n)),
                Some(Err(e)) => Err(format!("%Error opening flash:/{} ({})\n", n, e.ios_text())),
                None => Err(format!("%Error opening flash:/{} (No such device)\n", n)),
            },
            _ => Ok(Vec::new()),
        }
    }

    /// writes what came (or was read) to a local destination
    fn put_local(&mut self, ni: usize, dst: &Loc, data: &[u8], from: &str) -> Result<(), String> {
        match dst {
            Loc::Running => {
                let text = String::from_utf8_lossy(data).into_owned();
                self.ios_merge(ni, &text, from);
                Ok(())
            }
            Loc::Startup => {
                let text = String::from_utf8_lossy(data).into_owned();
                self.ios_set_startup(ni, &text);
                Ok(())
            }
            Loc::Flash(Some(n)) => match self.file_put(ni, n, data, false) {
                Ok(p) => {
                    self.out(ni, &format!("{} bytes copied to {}\n", data.len(), p));
                    Ok(())
                }
                Err(e) => Err(format!("%Error writing flash:/{} ({})\n", n, e.ios_text())),
            },
            _ => Ok(()),
        }
    }

    /// runs a copy whose questions are answered
    fn files_run(&mut self, ni: usize, d: Dialog) {
        let r = self.files_copy(ni, d);
        if let Err(e) = r {
            self.out(ni, &e);
        }
    }

    fn files_copy(&mut self, ni: usize, d: Dialog) -> Result<(), String> {
        if let Loc::Server { host: Some(h), .. } = d.src.clone().or_dst(&d.dst)
            && let Some(c) = self.cdev_mut(ni)
        {
            c.files.last_host = h;
        }
        match (&d.src, &d.dst) {
            (Loc::Server { ftp, user, pass, host: Some(h), file: Some(f) }, dst) => {
                let sink = match dst {
                    Loc::Running => Sink::Running,
                    Loc::Startup => Sink::Startup,
                    Loc::Flash(Some(n)) => {
                        // the space is checked before the transfer starts only as far as known
                        Sink::File(format!("flash:/{}", n))
                    }
                    _ => return Err(String::from("%Error: no destination\n")),
                };
                if *ftp {
                    let (u, p) = self.ftp_login(ni, user, pass);
                    let c = crate::ftpc::FtpProgram::ios_get(h, f, &u, &p, sink);
                    self.run_program(ni, "ftp", Box::new(c), false).map_err(|e| format!("{}\n", e))
                } else {
                    let c = TftpClient::get(h, f, sink, Style::Ios);
                    self.run_program(ni, "tftp", Box::new(c), false).map_err(|e| format!("{}\n", e))
                }
            }
            (src, Loc::Server { ftp, user, pass, host: Some(h), file: Some(f) }) => {
                let data = self.src_bytes(ni, src)?;
                if *ftp {
                    let (u, p) = self.ftp_login(ni, user, pass);
                    let c = crate::ftpc::FtpProgram::ios_put(h, f, &u, &p, data);
                    self.run_program(ni, "ftp", Box::new(c), false).map_err(|e| format!("{}\n", e))
                } else {
                    let c = TftpClient::put(h, f, data, Style::Ios);
                    self.run_program(ni, "tftp", Box::new(c), false).map_err(|e| format!("{}\n", e))
                }
            }
            (src, dst) => {
                let data = self.src_bytes(ni, src)?;
                let from = match src {
                    Loc::Flash(Some(n)) => format!("flash:/{}", n),
                    Loc::Startup => String::from("nvram:/startup-config"),
                    _ => String::from("system:running-config"),
                };
                self.put_local(ni, dst, &data, &from)
            }
        }
    }

    /// the FTP login of an IOS-like client: the URL's, else "ip ftp username / password", else
    /// anonymous
    fn ftp_login(&self, ni: usize, user: &Option<String>, pass: &Option<String>) -> (String, String) {
        let c = self.cdev(ni);
        let u = user.clone().filter(|u| !u.is_empty()).or_else(|| c.map(|c| c.files.ftp_user.clone()).filter(|u| !u.is_empty())).unwrap_or_else(|| String::from("anonymous"));
        let p = pass.clone().filter(|p| !p.is_empty()).or_else(|| c.map(|c| c.files.ftp_pass.clone()).filter(|p| !p.is_empty())).unwrap_or_else(|| format!("{}@", self.host(ni).to_ascii_lowercase()));
        (u, p)
    }

    /// a configuration merged into the running one, as typed in configuration mode: it adds
    /// and changes what it names (as IOS: an interface it names without "no shutdown" stays as
    /// it is)
    pub(crate) fn ios_merge(&mut self, ni: usize, text: &str, from: &str) {
        let mode = self.cdev(ni).map(|c| c.mode).unwrap_or(Mode::Priv);
        if let Some(c) = self.cdev_mut(ni) {
            c.mode = Mode::Config;
        }
        let mut errors = 0;
        for raw in text.lines() {
            let t = raw.trim();
            if t.is_empty() || t.starts_with('!') || t.starts_with('#') {
                continue;
            }
            if t.eq_ignore_ascii_case("end") {
                break;
            }
            // EXEC commands would wait on the console: they are not for a configuration
            if t.to_ascii_lowercase().starts_with("do ") {
                continue;
            }
            if self.cli_run(ni, t).is_err() {
                errors += 1;
            }
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.mode = mode;
            c.ifs.clear();
            c.vlans.clear();
        }
        self.ios_snap(ni);
        if errors > 0 {
            self.out(ni, &format!("{} lines of {} were not taken\n", errors, from));
        }
        self.ios_log(ni, &format!("%SYS-5-CONFIG_I: configuration changed from {}", from));
    }

    /// a configuration becomes the startup configuration
    pub(crate) fn ios_set_startup(&mut self, ni: usize, text: &str) {
        let dev = self.dev_of(ni);
        if let Some(c) = self.cdev_mut(ni) {
            c.cfg.startup = String::from(text);
        }
        self.out(ni, "Saved as the startup configuration.\n");
        self.cli.reports.push(Report::Startup { dev, text: String::from(text) });
    }

    // ================================================================ Linux

    /// an ls -l line
    fn ls_line(name: &str, s: &Stat) -> String {
        let (mode, links, size) = if s.dir { ("drwxr-xr-x", 2, 4096) } else { ("-rw-r--r--", 1, s.size) };
        format!("{} {} root root {:>5} {} {}\n", mode, links, size, ls_date(s.mtime), name)
    }

    /// the entries of a directory for ls (the made files of /etc included)
    fn ls_entries(&self, ni: usize, p: &str) -> Result<Vec<(String, Stat)>, FsErr> {
        let st = self.store(ni).ok_or(FsErr::NotFound)?;
        let mut v = st.list(p)?;
        if p == "/etc" {
            for e in ETC {
                let size = self.cat_etc(ni, &format!("/etc/{}", e)).map_or(0, |t| t.len() as u64);
                v.push((String::from(e), Stat { size, mtime: 0, dir: false, builtin: true }));
            }
            v.sort_by(|a, b| a.0.cmp(&b.0));
        }
        Ok(v)
    }

    /// the files of /etc the shell makes (netsvc.rs)
    fn cat_etc(&self, ni: usize, p: &str) -> Option<String> {
        match p {
            "/etc/resolv.conf" => Some(self.resolv_text(ni)),
            "/etc/hosts" => Some(self.hosts_text(ni)),
            "/etc/hostname" => Some(format!("{}\n", self.host(ni))),
            _ => None,
        }
    }

    pub(crate) fn sh_ls(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut long = false;
        let mut all = false;
        let mut paths: Vec<&str> = Vec::new();
        for a in args {
            match *a {
                s if s.starts_with('-') && s.len() > 1 => {
                    for c in s[1..].chars() {
                        match c {
                            'l' => long = true,
                            'a' | 'A' => all = true,
                            'h' | '1' => {}
                            o => return self.sh_err(ni, &format!("ls: invalid option -- '{}'\n", o)),
                        }
                    }
                }
                s => paths.push(s),
            }
        }
        if paths.is_empty() {
            paths.push(".");
        }
        let Some(st) = self.store(ni).cloned() else { return self.sh_err(ni, "ls: no file system\n") };
        let mut out = String::new();
        let mut err: Option<String> = None;
        let several = paths.len() > 1;
        for (k, path) in paths.iter().enumerate() {
            let p = match st.norm(path) {
                Ok(p) => p,
                Err(e) => {
                    let m = format!("ls: cannot access '{}': {}\n", path, e.errno_text());
                    out.push_str(&m);
                    err.get_or_insert(m);
                    continue;
                }
            };
            let etc = self.cat_etc(ni, &p).map(|t| Stat { size: t.len() as u64, mtime: 0, dir: false, builtin: true });
            let Some(s) = st.stat(&p).or(etc) else {
                let m = format!("ls: cannot access '{}': No such file or directory\n", path);
                out.push_str(&m);
                err.get_or_insert(m);
                continue;
            };
            if !s.dir {
                out.push_str(&if long { Engine::ls_line(path, &s) } else { format!("{}\n", path) });
                continue;
            }
            if several {
                if k > 0 {
                    out.push('\n');
                }
                out.push_str(&format!("{}:\n", path));
            }
            let mut v = self.ls_entries(ni, &p).unwrap_or_default();
            if all {
                let dot = Stat { size: 0, mtime: 0, dir: true, builtin: true };
                v.insert(0, (String::from(".."), dot));
                v.insert(0, (String::from("."), dot));
            }
            if long {
                let blocks: u64 = v.iter().map(|(_, s)| if s.dir { 4 } else { s.size.div_ceil(4096) * 4 }).sum();
                out.push_str(&format!("total {}\n", blocks));
                for (n, s) in &v {
                    out.push_str(&Engine::ls_line(n, s));
                }
            } else if !v.is_empty() {
                let names: Vec<&str> = v.iter().map(|(n, _)| n.as_str()).collect();
                out.push_str(&names.join("  "));
                out.push('\n');
            }
        }
        self.out(ni, &out);
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    /// cat of a file of the store (sh_cat asks after the files of /etc)
    pub(crate) fn cat_file(&self, ni: usize, path: &str) -> Result<String, String> {
        let st = self.store(ni).ok_or_else(|| format!("cat: {}: No such file or directory\n", path))?;
        match st.read(path) {
            Ok(b) => Ok(String::from_utf8_lossy(b).into_owned()),
            Err(e) => Err(format!("cat: {}: {}\n", path, e.errno_text())),
        }
    }

    pub(crate) fn sh_rm(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut force = false;
        let mut rec = false;
        let mut files: Vec<&str> = Vec::new();
        for a in args {
            match *a {
                s if s.starts_with('-') && s.len() > 1 => {
                    for c in s[1..].chars() {
                        match c {
                            'f' => force = true,
                            'r' | 'R' => rec = true,
                            'v' | 'i' => {}
                            o => return self.sh_err(ni, &format!("rm: invalid option -- '{}'\n", o)),
                        }
                    }
                }
                s => files.push(s),
            }
        }
        if files.is_empty() {
            return if force { Ok(()) } else { self.sh_err(ni, "rm: missing operand\n") };
        }
        let mut err = None;
        for f in files {
            match self.file_del(ni, f, rec) {
                Ok(_) => {}
                Err(FsErr::NotFound) if force => {}
                Err(e) => {
                    let m = format!("rm: cannot remove '{}': {}\n", f, if e == FsErr::ReadOnly { "Permission denied" } else { e.errno_text() });
                    self.out(ni, &m);
                    err.get_or_insert(m);
                }
            }
        }
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_cp(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let ops: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-') || a.len() == 1).collect();
        if ops.len() < 2 {
            return self.sh_err(ni, if ops.is_empty() { "cp: missing file operand\n" } else { "cp: missing destination file operand\n" });
        }
        let Some(st) = self.store(ni).cloned() else { return self.sh_err(ni, "cp: no file system\n") };
        let (src, dst) = (ops[0], ops[ops.len() - 1]);
        let data = match self.cat_etc(ni, &st.norm(src).unwrap_or_default()) {
            Some(t) => t.into_bytes(),
            None => match st.read(src) {
                Ok(b) => Vec::from(b),
                Err(FsErr::IsDir) => return self.sh_err(ni, &format!("cp: -r not specified; omitting directory '{}'\n", src)),
                Err(e) => return self.sh_err(ni, &format!("cp: cannot stat '{}': {}\n", src, e.errno_text())),
            },
        };
        let target = if st.is_dir(dst) {
            let name = st.norm(src).map(|p| String::from(st.name_of(&p))).unwrap_or_default();
            format!("{}/{}", st.norm(dst).unwrap_or_default().trim_end_matches('/'), name)
        } else {
            String::from(dst)
        };
        match self.file_put(ni, &target, &data, false) {
            Ok(_) => Ok(()),
            Err(e) => self.sh_err(ni, &format!("cp: cannot create regular file '{}': {}\n", dst, e.errno_text())),
        }
    }

    pub(crate) fn sh_mkdir(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let parents = args.iter().any(|a| *a == "-p");
        let dirs: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
        if dirs.is_empty() {
            return self.sh_err(ni, "mkdir: missing operand\n");
        }
        let mut err = None;
        for d in dirs {
            if let Err(e) = self.file_mkdir(ni, d, parents) {
                let m = format!("mkdir: cannot create directory '{}': {}\n", d, e.errno_text());
                self.out(ni, &m);
                err.get_or_insert(m);
            }
        }
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_rmdir(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dirs: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
        if dirs.is_empty() {
            return self.sh_err(ni, "rmdir: missing operand\n");
        }
        let mut err = None;
        for d in dirs {
            if let Err(e) = self.file_rmdir(ni, d) {
                let m = format!("rmdir: failed to remove '{}': {}\n", d, e.errno_text());
                self.out(ni, &m);
                err.get_or_insert(m);
            }
        }
        match err {
            Some(e) => Err(String::from(e.trim_end())),
            None => Ok(()),
        }
    }

    pub(crate) fn sh_touch(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        for f in args.iter().filter(|a| !a.starts_with('-')) {
            let have = self.store(ni).and_then(|s| s.read(f).ok().map(Vec::from));
            let data = have.unwrap_or_default();
            if let Err(e) = self.file_put(ni, f, &data, false) {
                return self.sh_err(ni, &format!("touch: cannot touch '{}': {}\n", f, e.errno_text()));
            }
        }
        Ok(())
    }

    /// "echo WORDS > FILE" / ">> FILE": the words (with their line end) into a file
    pub(crate) fn sh_echo(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut words: Vec<&str> = Vec::new();
        let mut target: Option<(String, bool)> = None;
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if let Some(rest) = a.strip_prefix(">>").or_else(|| a.strip_prefix('>')) {
                let append = a.starts_with(">>");
                let f = if rest.is_empty() {
                    i += 1;
                    args.get(i).copied().unwrap_or("")
                } else {
                    rest
                };
                target = Some((String::from(f), append));
            } else {
                words.push(a);
            }
            i += 1;
        }
        let (newline, words) = match words.first() {
            Some(&"-n") => (false, &words[1..]),
            _ => (true, &words[..]),
        };
        let mut text = words.join(" ");
        if newline {
            text.push('\n');
        }
        match target {
            None => {
                self.out(ni, &text);
                Ok(())
            }
            Some((f, _)) if f.is_empty() => self.sh_err(ni, "jfn-sh: syntax error near unexpected token `newline'\n"),
            // the resolver's settings (netsvc.rs)
            Some((f, append)) if self.store(ni).and_then(|s| s.norm(&f).ok()).is_some_and(|p| p == "/etc/resolv.conf") => self.resolv_write(ni, &text, append),
            Some((f, append)) => match self.file_put(ni, &f, text.as_bytes(), append) {
                Ok(_) => Ok(()),
                Err(e) => self.sh_err(ni, &format!("jfn-sh: {}: {}\n", f, e.errno_text())),
            },
        }
    }

    /// tftp -g|-p [-l LOCAL] [-r REMOTE] [-b SIZE] HOST [PORT] (busybox-like)
    pub(crate) fn sh_tftp(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        const USAGE: &str = "Usage: tftp [-g|-p] [-l LOCAL] [-r REMOTE] [-b SIZE] HOST [PORT]\n  Fetches (-g) or sends (-p) one file over TFTP.\n  -l FILE   the file here (- for the console)\n  -r FILE   the file on the server\n  -b SIZE   ask for blocks of SIZE bytes (RFC 2348)\n";
        let (mut get, mut put) = (false, false);
        let (mut local, mut remote, mut blk): (Option<String>, Option<String>, Option<u16>) = (None, None, None);
        let mut rest: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-g" => get = true,
                "-p" => put = true,
                "-l" | "-r" | "-b" => {
                    let Some(v) = args.get(i + 1) else { return self.sh_err(ni, USAGE) };
                    match args[i] {
                        "-l" => local = Some(String::from(*v)),
                        "-r" => remote = Some(String::from(*v)),
                        _ => match v.parse::<u16>() {
                            Ok(b) if (tftp_min()..=tftp_max()).contains(&b) => blk = Some(b),
                            _ => return self.sh_err(ni, &format!("tftp: bad block size '{}'\n", v)),
                        },
                    }
                    i += 1;
                }
                "-h" | "--help" => {
                    self.out(ni, USAGE);
                    return Ok(());
                }
                a if a.starts_with('-') => return self.sh_err(ni, &format!("tftp: invalid option -- '{}'\n{}", a.trim_start_matches('-'), USAGE)),
                a => rest.push(a),
            }
            i += 1;
        }
        if get == put || rest.is_empty() || rest.len() > 2 {
            return self.sh_err(ni, USAGE);
        }
        let host = rest[0];
        let port = match rest.get(1).map(|p| p.parse::<u16>()) {
            None => netpkt::udp::PORT_TFTP,
            Some(Ok(p)) if p > 0 => p,
            Some(_) => return self.sh_err(ni, &format!("tftp: bad port '{}'\n", rest[1])),
        };
        let mut c = if get {
            let Some(r) = remote.clone().or_else(|| local.clone()) else { return self.sh_err(ni, USAGE) };
            let l = local.unwrap_or_else(|| String::from(r.rsplit('/').next().unwrap_or(&r)));
            let sink = if l == "-" {
                Sink::Stdout
            } else {
                // the local file must be writable before anything goes out
                let st = self.store(ni);
                let ok = st.is_some_and(|s| s.norm(&l).is_ok_and(|p| s.is_dir(&s.parent(&p)) && !s.is_dir(&p)));
                if !ok {
                    let why = if st.is_some_and(|s| s.is_dir(&l)) { "Is a directory" } else { "No such file or directory" };
                    return self.sh_err(ni, &format!("tftp: can't open '{}': {}\n", l, why));
                }
                Sink::File(l)
            };
            TftpClient::get(host, &r, sink, Style::Linux)
        } else {
            let Some(l) = local.clone().or_else(|| remote.clone()) else { return self.sh_err(ni, USAGE) };
            let data = match self.store(ni).map(|s| s.read(&l).map(Vec::from)) {
                Some(Ok(d)) => d,
                Some(Err(e)) => return self.sh_err(ni, &format!("tftp: can't open '{}': {}\n", l, e.errno_text())),
                None => return self.sh_err(ni, "tftp: no file system\n"),
            };
            let r = remote.unwrap_or_else(|| String::from(l.rsplit('/').next().unwrap_or(&l)));
            TftpClient::put(host, &r, data, Style::Linux)
        };
        c.port = port;
        c.blksize = blk;
        self.run_program(ni, "tftp", Box::new(c), false)
    }
}

fn tftp_min() -> u16 {
    netpkt::tftp::BLKSIZE_MIN
}

fn tftp_max() -> u16 {
    netpkt::tftp::BLKSIZE_MAX
}

trait OrDst {
    fn or_dst(self, d: &Loc) -> Loc;
}

impl OrDst for Loc {
    /// a server among the two ends of a copy
    fn or_dst(self, d: &Loc) -> Loc {
        match self {
            Loc::Server { .. } => self,
            _ => d.clone(),
        }
    }
}
