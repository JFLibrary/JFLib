// policy -- the app's permissions (apps/NAME/PERMISSIONS, docs/PERMISSIONS.md): parsed once at
// start-up, then checked by the public file, process, shell, plugin and print functions
//
//   install(text) -> Result<Key, String>   parse and activate a manifest (once per process);
//                                          before that, and in apps without one, all is allowed
//   active() -> bool
//   fs_allowed(path, FS_READ | FS_WRITE) / process_allowed(program) / url_allowed(url) /
//   plugin_allowed(path) / print_allowed() / secrets_allowed(service) / autostart_allowed() /
//   audio_allowed()                        -> bool
//   input_global_allowed() / screen_capture_allowed()
//                                          -> bool
//                                          a refusal is recorded (denials) and printed once
//                                          per target to stderr: "PERMDENY kind target"
//   audio_granted() -> bool                sound is allowed now, without asking or recording a
//                                          refusal (ui's dialog sounds stay silent without it)
//   net_connect_allowed(addr) / net_listen_allowed(addr) / net_name_allowed(host) -> bool
//   net_resolved(host, addrs)              (resolvers) the addresses of a permitted name:
//                                          connections to them follow that name's rules
//   Key::grant(path, FS_*) / Key::grant_dir(dir, FS_*)   a file / folder the user handed over
//                                          (file dialog, drop, command line); needs fs.user
//   Key::trusted(f)                        f runs on this thread without the file checks (the
//                                          library's own file dialog and test scripts)
//   denials() -> Vec<Denial>               the last DENIALS_MAX refusals (audit)
//   normalize(path) -> Option<String>      absolute, '/' separators, no "." / ".." / empty
//                                          segments; Windows: upper-case drive letter, no
//                                          device, stream or reserved names; Linux: no ".."
//                                          at all (the kernel applies it after symbolic
//                                          links); None = refused
//   normalize_for(path, windows, cwd)      the same rules for either OS (tests, tools)
//   font_dirs() -> [folder]                the system font folders in scan order (fontdb)
//   cap_mask() -> CAP_* bits               what the policy leaves of capabilities()
//
// Questions to the user ("ask" lines; one decision per line):
//   questions() -> Vec<Question>           what waits for the user's answer (the UI shows it)
//   Key::ask_here(on)                      this thread (the UI's) asks: an access on it is
//                                          refused while its question is open (Verdict::
//                                          Pending; the UI tells the app when it was answered),
//                                          other threads wait up to Key::set_wait ms (env
//                                          jfgui_PERM_TIMEOUT, WAIT_MS) and are refused then.
//                                          Nobody asks (headless runs, console programs): refused
//   Key::answer(id, Answer)                Once / Deny: until the program ends; Always / Never:
//                                          also stored (Key::set_store, env jfgui_PERM_STORE,
//                                          STORE in config://; the app cannot write that file)
//   Key::set_answer(Some(a))               every question answered with a, without asking
//                                          (scripts; env jfgui_PERM_ANSWER=once|always|deny|never)
//   decisions() -> Vec<Decision>           every ask line and agent action with its answer
//   forget(rule) / forget_all()            revoke: asked again on the next access
//   reload()                               read the stored answers again
// Plugins (their own <stem>.permissions beside the library, same syntax without ask and agent
// lines, $PLUGIN = the plugin's folder; read access to that folder is implied):
//   plugin_rights(lib_path) -> Result<Arc<PluginRights>, String>
//   on_behalf(&rights, f)                  f on this thread for the plugin: every check must
//                                          pass the plugin's rules AND the app's (nested scopes:
//                                          all of them); jobs it starts on the pool keep it
//   plugin_scope() -> Option<Arc<PluginRights>>   the innermost scope of this thread
// Agent control (agent.confirm / agent.deny lines):
//   agent_rule(action) -> AgentRule        agent_ask(action, description) -> Verdict
//
// Manifest: one rule per line, '#' starts a comment, a key may repeat:
//   fs.read PATTERN..     read files below PATTERN
//   fs.write PATTERN..    write (and read) files below PATTERN
//   fs.user               files the user chooses (dialog, drop, command line): read + write;
//                         also every document the system hands over one at a time (Android's
//                         content://, a Flatpak's documents portal mount, the browser's
//                         /handles/), so they work after a restart too
//   process PATTERN|NAME..   programs Command::spawn / run_elevated may start (NAME: no '/')
//   url SCHEME..|*        open_url schemes (https, mailto, ...)
//   plugins PATTERN..     native libraries for lib_open
//   print                 plat::print
//   secrets               plat::secret (the system's store for passwords, tokens and keys)
//   autostart             plat::autostart_set (start the app at login; reading is free)
//   audio.output          plat::audio: streams, play / play_file, the system's sounds (sound)
//   input.global          plat::hotkey_register (global shortcuts: chords pressed while
//                         another program has the keyboard)
//   screen.capture        plat::screen::capture_start (pictures of the screen; picking one
//                         pixel's colour needs nothing: the user chooses it)
//   net.connect HOST:PORT..   TCP connections and UDP datagrams to HOST (a name, *.name for
//                         every name below it, an IPv4 / [IPv6] address, a block 10.0.0.0/8
//                         or [fd00::]/8, localhost = the loopback addresses, * = any) on PORT
//                         (a number, a range 8000-8099 or *); names are checked when they are
//                         resolved, their addresses then count for the name; with any name
//                         rule, port 53 (DNS) is open too
//   net.listen ADDR:PORT..    TCP listeners and UDP sockets with a fixed port on ADDR (an
//                         address, a block, localhost or *; 0.0.0.0 / [::] = every interface)
//                         and PORT (port 0 = any free one: the rule needs 0 or *)
//   ask KEY ARGS.. ["reason"]   (every key but fs.user) the first access asks the user; a
//                         plain rule that covers the access wins over an ask line
//   agent.confirm NAME..  agent actions (glob: '*' any run) the user confirms first
//   agent.deny NAME..     agent actions an agent may not run
//   sandbox [strict]      ask the OS to enforce the manifest as well (Landlock + seccomp on
//                         Linux, process mitigation policies on Windows; sandbox.rs); `strict`
//                         refuses to start when the OS cannot enforce it
//   recent                the host's list of recent files for a Flatpak (tools/pkg_linux.py:
//                         --filesystem=xdg-data/recently-used.xbel, the list of every program);
//                         at run time nothing changes: the system's list is always allowed
//                         for files the app may read (recent.rs)
// A PATTERN is an absolute path (X:/.. and //server/share/.. count on Windows only, /.. on
// Linux only), or starts with $CONFIG $TEMP $HOME $EXE $FONTS, the app's own folders
// $APPCONFIG $APPDATA $APPCACHE $APPLOG (vfs::app_dir) or the user's folders $DESKTOP
// $DOCUMENTS $DOWNLOADS $PICTURES $MUSIC $VIDEOS (user_dir; a variable without a value
// drops the pattern); '*' / '?' match inside one segment, a '**' segment any number of
// segments ("dir/**" also matches dir itself); "**" alone matches every path.  Windows
// compares without case.  Always readable: the font folders, the Hunspell dictionary folders
// (spell::hunspell_dirs), the exe folder and on Linux /etc/resolv.conf and /etc/hosts
// (netio's resolver); always writable: the app's own folders (config://, data://, cache://,
// logs://; vfs::set_app_id comes first) except the file of the stored answers.  trash counts
// as writing the path.
//
// The checks are lexical: a symbolic link inside an allowed folder can point outside it.  On
// Linux a path with a ".." segment is refused: "allowed/link/../x" would leave the folder.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use rt::sync::SpinLock;

/// read access ([`fs_allowed`], [`Key::grant`])
pub const FS_READ: u32 = 1;
/// write access; implies [`FS_READ`]
pub const FS_WRITE: u32 = 2;
/// refusals kept for [`denials`]
pub const DENIALS_MAX: usize = 64;
/// how long a thread other than the asking one waits for the user's answer by default (ms)
pub const WAIT_MS: u32 = 60_000;
/// where Always / Never answers are kept by default ([`Key::set_store`])
pub const STORE: &str = "config://permission-decisions.txt";

/// one refused access
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denial {
    /// "fs.read", "fs.write", "process", "url", "plugins", "print", "secrets", "autostart",
    /// "audio.output", "net.connect", "net.listen", "agent", "input.global" or "screen.capture"
    pub kind: &'static str,
    /// the path, program, URL, address, the service of a secret, the agent's action, the
    /// chord of a shortcut or "" (print, autostart, audio.output, screen.capture)
    pub target: String,
    /// the plugin the call was made for ("" = the app itself)
    pub plugin: String,
}

/// the user's answer to a question
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// allowed until the program ends
    Once,
    /// allowed, also in later runs (stored)
    Always,
    /// refused until the program ends
    Deny,
    /// refused, also in later runs (stored)
    Never,
}

impl Answer {
    /// Once or Always
    pub fn allows(self) -> bool {
        matches!(self, Answer::Once | Answer::Always)
    }
    /// Always or Never: kept for later runs
    pub fn stored(self) -> bool {
        matches!(self, Answer::Always | Answer::Never)
    }
    /// "once", "always", "deny", "never" (scripts, env, the store)
    pub fn word(self) -> &'static str {
        match self {
            Answer::Once => "once",
            Answer::Always => "always",
            Answer::Deny => "deny",
            Answer::Never => "never",
        }
    }
    /// the answer of word
    pub fn parse(w: &str) -> Option<Answer> {
        [Answer::Once, Answer::Always, Answer::Deny, Answer::Never].into_iter().find(|a| a.word() == w)
    }
}

/// the outcome of a check that may ask the user
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// go ahead
    Allow,
    /// refused (the rules, the user's answer, nobody to ask, a timeout)
    Deny,
    /// refused for now: the question waits for the user on this (the asking) thread
    Pending,
}

/// a question waiting for the user's answer
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// for [`Key::answer`]
    pub id: u32,
    /// the kind of access ("fs.read", "net.connect", ..., "agent")
    pub kind: &'static str,
    /// what is accessed: the path, host, program, URL or the agent's action
    pub target: String,
    /// the ask line the answer decides ("fs.read $DOCUMENTS/**", "agent document.delete")
    pub rule: String,
    /// the manifest's reason ("" without one); an agent action: its description
    pub reason: String,
    /// the plugin the call is made for ("" = the app)
    pub plugin: String,
}

/// an ask line or an agent action with its answer ([`decisions`])
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// the ask line ("url mailto") or "agent ACTION"
    pub rule: String,
    /// its kind ("url", ..., "agent")
    pub kind: &'static str,
    /// the manifest's reason or the action's description ("" when unknown)
    pub reason: String,
    /// None: not answered (asked on the next access)
    pub answer: Option<Answer>,
}

/// what the manifest says about an agent action ([`agent_rule`])
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentRule {
    /// no rule: the agent may run it
    Free,
    /// agent.confirm: the user confirms it first
    Confirm,
    /// agent.deny: never for an agent
    Deny,
}

// the kinds of access (Denial::kind, Question::kind)
const KINDS: [&str; 14] = [
    "fs.read", "fs.write", "process", "url", "plugins", "print", "secrets", "autostart", "audio.output", "net.connect",
    "net.listen", "agent", "input.global", "screen.capture",
];

fn kind_of(rule: &str) -> &'static str {
    let w = rule.split(' ').next().unwrap_or("");
    KINDS.iter().find(|k| **k == w).copied().unwrap_or("")
}

// every rule carries an ask tag: 0 = allowed, k = the manifest's ask line k (Policy::asks[k - 1])
struct Rule {
    segs: Vec<String>,
    mode: u32,
    ask: u16,
}

// an address a permitted name resolved to: the port range and ask tag of the name's rule, the
// name (plugins check it against their own rules)
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
struct Resolved {
    ip: [u8; 16],
    lo: u16,
    hi: u16,
    ask: u16,
    name: String,
}

#[derive(Default)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
struct Policy {
    fs: Vec<Rule>,
    /// the user's files (Key::grant, with fs.user)
    granted: Vec<Rule>,
    explicit_read: bool,
    explicit_write: bool,
    user: bool,
    process: Vec<(Vec<String>, u16)>,
    names: Vec<(String, u16)>,
    urls: Vec<(String, u16)>,
    plugins: Vec<(Vec<String>, u16)>,
    /// None: not allowed, Some(0): allowed, Some(k): ask line k
    print: Option<u16>,
    secrets: Option<u16>,
    autostart: Option<u16>,
    audio: Option<u16>,
    hotkeys: Option<u16>,
    capture: Option<u16>,
    connect: Vec<NetRule>,
    listen: Vec<NetRule>,
    resolved: Vec<Resolved>,
    /// the ask lines: (rule, reason)
    asks: Vec<(String, String)>,
    agent_confirm: Vec<String>,
    agent_deny: Vec<String>,
    /// OS sandbox the manifest asks for: 0 none, 1 `sandbox`, 2 `sandbox strict` (sandbox.rs)
    sandbox: u8,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static KEY_GIVEN: AtomicBool = AtomicBool::new(false);
static POLICY: SpinLock<Option<Policy>> = SpinLock::new(None);
static DENIALS: SpinLock<Vec<Denial>> = SpinLock::new(Vec::new());
// threads inside Key::trusted: thread id + 1, 0 = free
const BYPASS_SLOTS: usize = 4;
static BYPASS: [AtomicU64; BYPASS_SLOTS] = [const { AtomicU64::new(0) }; BYPASS_SLOTS];

/// The right to widen the installed policy for the user's own files, to run trusted library
/// code and to answer questions; only [`install`] creates one (the UI runtime keeps it to
/// itself).
#[derive(Clone)]
pub struct Key(());

/// parse and activate a manifest; Err names the first bad line.  Only the first call of a
/// process can succeed.
pub fn install(text: &str) -> Result<Key, String> {
    let p = parse(text, "PERMISSIONS", None)?;
    let mut slot = POLICY.lock();
    if slot.is_some() {
        return Err(String::from("a policy is already installed"));
    }
    *slot = Some(p);
    ACTIVE.store(true, Ordering::SeqCst);
    KEY_GIVEN.store(true, Ordering::SeqCst);
    Ok(Key(()))
}

/// the key of a process without a manifest (it answers the questions of agent actions); only
/// the first call (and none after install) gets it
pub fn take_key() -> Option<Key> {
    (!KEY_GIVEN.swap(true, Ordering::SeqCst)).then_some(Key(()))
}

/// whether a policy restricts this process
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// whether path may be read (FS_READ) or written (FS_WRITE)
pub fn fs_allowed(path: &str, mode: u32) -> bool {
    if !active() && SCOPED.load(Ordering::Acquire) == 0 {
        return true;
    }
    if !scoped_here() && (!active() || bypassed()) {
        return true;
    }
    let kind = if mode & FS_WRITE != 0 { "fs.write" } else { "fs.read" };
    let Some(n) = normalize(path) else {
        record(kind, path, &scope_name(), true);
        return false;
    };
    let segs = split(&fold(&n));
    if mode & FS_WRITE != 0 && active() && store_protects(&segs) {
        record(kind, path, &scope_name(), true); // the stored answers are the user's
        return false;
    }
    // documents the system hands over one at a time, like Android's content://: with fs.user
    if user_doc_path(&n) && scopes_here().is_empty() && POLICY.lock().as_ref().is_some_and(|p| p.user) {
        return true;
    }
    decide(kind, path, &Req::Fs(&segs, mode), true) == Verdict::Allow
}

// a normalised path of such a document: Flatpak's documents portal inside the sandbox (it shows
// only what was granted to the app; linux_docs.rs), the browser's files of the user (a handle
// each, web_files.rs)
#[cfg(target_os = "linux")]
fn user_doc_path(n: &str) -> bool {
    crate::linux_docs::sandboxed(n)
}
#[cfg(target_arch = "wasm32")]
fn user_doc_path(n: &str) -> bool {
    n.starts_with("/handles/")
}
#[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
fn user_doc_path(_: &str) -> bool {
    false
}

/// whether the app may reach a document the user chose by its URI (Android's content://
/// names; the system grants each one): with fs.user in the manifest, or without a manifest
pub fn user_doc_allowed(uri: &str) -> bool {
    if !active() || bypassed() {
        return true;
    }
    if POLICY.lock().as_ref().is_some_and(|p| p.user) {
        return true;
    }
    record("fs.user", uri, &scope_name(), true);
    false
}

/// whether Command::spawn / run_elevated may start program (a path, or a name found in PATH)
pub fn process_allowed(program: &str) -> bool {
    let req = if program.contains(['/', '\\']) {
        match normalize(program) {
            Some(n) => Req::ProcPath(split(&fold(&n))),
            None => Req::Never,
        }
    } else {
        Req::ProcName(fold(program))
    };
    decide("process", program, &req, false) == Verdict::Allow
}

/// whether open_url may open url (its scheme)
pub fn url_allowed(url: &str) -> bool {
    // control characters and over-long URLs (the shell buffer would truncate them) are refused
    if url.chars().any(|c| c < ' ' || c == '\x7f') || url.encode_utf16().count() >= 2048 {
        return false;
    }
    let scheme = match url.find(':') {
        Some(k) if k > 0 => url[..k].to_ascii_lowercase(),
        _ => String::new(),
    };
    decide("url", url, &Req::Url(&scheme), false) == Verdict::Allow
}

/// whether lib_open may load the native library at path
pub fn plugin_allowed(path: &str) -> bool {
    let req = match normalize(path) {
        Some(n) => Req::Plugin(split(&fold(&n))),
        None => Req::Never,
    };
    decide("plugins", path, &req, false) == Verdict::Allow
}

/// whether plat::print may print
pub fn print_allowed() -> bool {
    decide("print", "", &Req::Print, false) == Verdict::Allow
}

/// whether plat::secret may use the system's secret store (`secrets`); a refusal names the
/// service ("" for secret_available)
pub fn secrets_allowed(service: &str) -> bool {
    decide("secrets", service, &Req::Secrets, false) == Verdict::Allow
}

/// whether plat::autostart_set may change the app's autostart entry (`autostart`)
pub fn autostart_allowed() -> bool {
    decide("autostart", "", &Req::Autostart, false) == Verdict::Allow
}

/// whether plat::audio may play sound (`audio.output`): streams, play, play_file, the
/// system's sounds
pub fn audio_allowed() -> bool {
    decide("audio.output", "", &Req::Audio, false) == Verdict::Allow
}

/// whether sound is allowed now without a question or a recorded refusal: no manifest, a plain
/// `audio.output` line or an ask line already answered with yes (ui's dialog sounds)
pub fn audio_granted() -> bool {
    granted(&Req::Audio)
}

// the quiet form of decide: the plugin scopes of this thread and the app's rules, an ask line
// only by its answer; nothing is recorded, nobody is asked
fn granted(req: &Req) -> bool {
    let scopes = scopes_here();
    if scopes.iter().any(|s| s.p.as_ref().is_some_and(|pp| pp.grant(req, &[], &[], false) != Grant::Allow)) {
        return false;
    }
    if !active() {
        return true;
    }
    let rule = {
        let g = POLICY.lock();
        let Some(p) = g.as_ref() else { return true };
        match p.grant(req, &p.granted, &p.resolved, true) {
            Grant::Allow => return true,
            Grant::No => return false,
            Grant::Ask(k) => p.asks.get(k as usize - 1).map(|a| a.0.clone()),
        }
    };
    rule.is_some_and(|r| decided(&r) == Some(true))
}

/// whether plat::hotkey_register may register global shortcuts (`input.global`)
pub fn input_global_allowed() -> bool {
    decide("input.global", "", &Req::Hotkeys, false) == Verdict::Allow
}

/// whether plat::screen::capture_start may take pictures of the screen (`screen.capture`)
pub fn screen_capture_allowed() -> bool {
    decide("screen.capture", "", &Req::Capture, false) == Verdict::Allow
}

/// the last refusals, oldest first
pub fn denials() -> Vec<Denial> {
    DENIALS.lock().clone()
}

/// the OS sandbox the manifest asks for: 0 none, 1 `sandbox`, 2 `sandbox strict` (sandbox.rs)
pub fn sandbox_level() -> u8 {
    POLICY.lock().as_ref().map_or(0, |p| p.sandbox)
}

// the literal leading path of a rule's segments (up to the first glob) as a Linux path ("/"
// when the rule starts with a glob), and whether the rule goes on below it (then it is a
// folder); for the OS sandbox, which is coarser than the lexical policy -- the fine-grained
// check stays in `plat`
#[cfg(target_os = "linux")]
fn literal_prefix(segs: &[String]) -> (String, bool) {
    let mut out: Vec<&str> = Vec::new();
    for s in segs {
        if s == "**" || s.contains(['*', '?']) {
            break;
        }
        out.push(s);
    }
    let below = out.len() < segs.len();
    // segs[0] is "" (the root): [""] / ["", "usr"] -> "/" / "/usr"
    let path = if out.len() <= 1 { String::from("/") } else { out.join("/") };
    (path, below)
}

/// what the Linux OS sandbox grants, derived from the active policy ([`sandbox_plan_linux`])
#[cfg(target_os = "linux")]
pub struct LinuxPlan {
    /// 0 none, 1 `sandbox`, 2 `sandbox strict`
    pub level: u8,
    /// literal folders / files the manifest reaches, with whether they may be written
    pub fs: Vec<(String, bool)>,
    /// writable folders to create when missing (the app's own folders, the folders of fs.write
    /// rules): the kernel can only grant paths that exist
    pub make_dirs: Vec<String>,
    /// the manifest names at least one program (execve is denied otherwise)
    pub exec_allowed: bool,
    /// restrict TCP connect to `connect_ports` (false: leave it unrestricted -- a `*` rule)
    pub connect_restrict: bool,
    /// the ports of net.connect (and 53 with a name rule); empty: no TCP connection at all
    pub connect_ports: Vec<u16>,
    /// restrict TCP bind to `listen_ports`
    pub listen_restrict: bool,
    /// the ports of net.listen; empty: no TCP listener at all
    pub listen_ports: Vec<u16>,
}

#[cfg(target_os = "linux")]
fn add_fs(fs: &mut Vec<(String, bool)>, path: String, w: bool) {
    if path.is_empty() {
        return;
    }
    match fs.iter_mut().find(|(q, _)| *q == path) {
        Some(e) => e.1 |= w,
        None => fs.push((path, w)),
    }
}

// the ports of net rules, bounded -> (restrict, ports): (false, _) for a wide range or a `*`
// port (leave TCP to the in-process policy), else the explicit ports (empty: deny every port)
#[cfg(target_os = "linux")]
fn plan_ports(rules: &[NetRule], dns: bool) -> (bool, Vec<u16>) {
    let mut ports: Vec<u16> = Vec::new();
    for r in rules {
        if r.hi as u32 - r.lo as u32 > 64 || ports.len() > 128 {
            return (false, Vec::new()); // a wide range: leave it to the in-process policy
        }
        for p in r.lo..=r.hi {
            if !ports.contains(&p) {
                ports.push(p);
            }
        }
    }
    if dns && !ports.contains(&53) {
        ports.push(53);
    }
    (true, ports)
}

/// what the Linux OS sandbox should enforce for the active policy (no policy: nothing)
#[cfg(target_os = "linux")]
pub fn sandbox_plan_linux() -> LinuxPlan {
    let g = POLICY.lock();
    let Some(p) = g.as_ref() else {
        return LinuxPlan {
            level: 0,
            fs: Vec::new(),
            make_dirs: Vec::new(),
            exec_allowed: true,
            connect_restrict: false,
            connect_ports: Vec::new(),
            listen_restrict: false,
            listen_ports: Vec::new(),
        };
    };
    let mut fs: Vec<(String, bool)> = Vec::new();
    let mut make_dirs: Vec<String> = Vec::new();
    for r in &p.fs {
        let (path, below) = literal_prefix(&r.segs);
        let w = r.mode & FS_WRITE != 0;
        if w && below && !make_dirs.contains(&path) {
            make_dirs.push(path.clone());
        }
        add_fs(&mut fs, path, w);
    }
    for r in &p.granted {
        add_fs(&mut fs, literal_prefix(&r.segs).0, r.mode & FS_WRITE != 0); // the user's files
    }
    for (segs, _) in &p.plugins {
        add_fs(&mut fs, literal_prefix(segs).0, false); // mapped executable: read + execute
    }
    let dns = p.connect.iter().any(|r| matches!(r.host, Host::Any | Host::Name(_)));
    let (connect_restrict, connect_ports) = plan_ports(&p.connect, dns);
    let (listen_restrict, listen_ports) = plan_ports(&p.listen, false);
    LinuxPlan {
        level: p.sandbox,
        fs,
        make_dirs,
        exec_allowed: !p.process.is_empty() || !p.names.is_empty(),
        connect_restrict,
        connect_ports,
        listen_restrict,
        listen_ports,
    }
}

/// what the Windows OS sandbox needs to know from the active policy ([`sandbox_plan_win`])
#[cfg(windows)]
pub struct WinPlan {
    /// 0 none, 1 `sandbox`, 2 `sandbox strict`
    pub level: u8,
    /// the manifest allows native plugins (dynamic code must stay allowed for the loader)
    pub plugins_allowed: bool,
    /// child processes may be created: the manifest names a program, or has a `url` / `print`
    /// rule (open_url's ShellExecute and the print dialog can start another program)
    pub exec_allowed: bool,
}

/// what the Windows OS sandbox should enforce for the active policy (no policy: nothing)
#[cfg(windows)]
pub fn sandbox_plan_win() -> WinPlan {
    let g = POLICY.lock();
    match g.as_ref() {
        Some(p) => WinPlan {
            level: p.sandbox,
            plugins_allowed: !p.plugins.is_empty(),
            exec_allowed: !p.process.is_empty() || !p.names.is_empty() || !p.urls.is_empty() || p.print.is_some(),
        },
        None => WinPlan { level: 0, plugins_allowed: true, exec_allowed: true },
    }
}

/// the CAP_* bits the policy leaves (all without a policy)
pub fn cap_mask() -> u64 {
    if !active() {
        return u64::MAX;
    }
    let g = POLICY.lock();
    let Some(p) = g.as_ref() else { return u64::MAX };
    let mut m = u64::MAX;
    if !(p.explicit_read || p.explicit_write || p.user) {
        m &= !crate::CAP_FILE_READ;
    }
    if !(p.explicit_write || p.user) {
        m &= !(crate::CAP_FILE_WRITE | crate::CAP_PERSISTENT_STORAGE);
    }
    if p.print.is_none() {
        m &= !crate::CAP_PRINT;
    }
    if p.plugins.is_empty() {
        m &= !crate::CAP_NATIVE_PLUGINS;
    }
    m
}

impl Key {
    /// let the app read (FS_READ) or read and write (FS_WRITE) the file at path, which the
    /// user chose; false without fs.user or for a path normalize refuses
    pub fn grant(&self, path: &str, mode: u32) -> bool {
        self.add(path, mode, false)
    }

    /// the same for a folder and everything below it
    pub fn grant_dir(&self, dir: &str, mode: u32) -> bool {
        self.add(dir, mode, true)
    }

    fn add(&self, path: &str, mode: u32, dir: bool) -> bool {
        let Some(n) = normalize(path) else { return false };
        let mut segs = split(&fold(&n));
        if dir {
            segs.push(String::from("**"));
        }
        let mut g = POLICY.lock();
        let Some(p) = g.as_mut() else { return false };
        if !p.user {
            return false;
        }
        if !p.granted.iter().any(|r| r.mode == mode && r.segs == segs) {
            p.granted.push(Rule { segs, mode, ask: 0 });
        }
        true
    }

    /// run f on this thread without the file checks
    pub fn trusted<R>(&self, f: impl FnOnce() -> R) -> R {
        let me = tid() + 1;
        if BYPASS.iter().any(|b| b.load(Ordering::Acquire) == me) {
            return f(); // nested
        }
        let slot = BYPASS.iter().find(|b| b.compare_exchange(0, me, Ordering::AcqRel, Ordering::Relaxed).is_ok());
        let r = f();
        if let Some(b) = slot {
            b.store(0, Ordering::Release);
        }
        r
    }

    /// this thread (the UI's) shows the questions (on) or nobody does (off: the open
    /// questions are withdrawn, waiting threads are refused)
    pub fn ask_here(&self, on: bool) {
        ASKER.store(if on { tid() + 1 } else { 0 }, Ordering::Release);
        if !on {
            QUESTIONS.lock().clear();
            bump();
        }
    }

    /// the user's answer to question id -> false when it is no longer open.  The answer
    /// decides the question's ask line; waiting threads continue.
    pub fn answer(&self, id: u32, a: Answer) -> bool {
        let q = {
            let mut g = QUESTIONS.lock();
            let Some(i) = g.iter().position(|q| q.id == id) else { return false };
            g.remove(i)
        };
        decide_rule(&q.rule, a, &q.reason);
        bump();
        true
    }

    /// answer every question with a, without asking (None: ask; jfgui_PERM_ANSWER)
    pub fn set_answer(&self, a: Option<Answer>) {
        AUTO.store(a.map_or(AUTO_NONE, |a| a as u8), Ordering::Release);
    }

    /// how long a thread other than the asking one waits for an answer (ms)
    pub fn set_wait(&self, ms: u32) {
        WAIT.store(ms, Ordering::Release);
    }

    /// where Always / Never answers are kept (None: in memory only); the answers of the new
    /// place are read, the ones of this run stay
    pub fn set_store(&self, path: Option<&str>) {
        let mut s = STORE_S.lock();
        s.path = Some(path.map(String::from));
        s.loaded = false;
    }
}

fn bypassed() -> bool {
    let me = tid() + 1;
    BYPASS.iter().any(|b| b.load(Ordering::Acquire) == me)
}

/// (task.rs) this thread runs inside Key::trusted: work it hands to the blocking pool keeps that
pub(crate) fn trusted_here() -> bool {
    active() && bypassed()
}

/// (task.rs) f without the file checks on this thread (a pool job of a trusted caller)
pub(crate) fn trusted_scope<R>(f: impl FnOnce() -> R) -> R {
    Key(()).trusted(f)
}

#[cfg(any(windows, target_os = "linux"))]
fn tid() -> u64 {
    crate::plat_tid()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn tid() -> u64 {
    0
}

// record a refusal (printed once per kind, target and plugin on stderr when print)
fn record(kind: &'static str, target: &str, plugin: &str, print: bool) {
    let d = Denial { kind, target: String::from(target), plugin: String::from(plugin) };
    let mut g = DENIALS.lock();
    let seen = g.contains(&d);
    if g.len() == DENIALS_MAX {
        g.remove(0);
    }
    g.push(d);
    drop(g);
    if !seen && print {
        let sep = if target.is_empty() { "" } else { " " };
        let of = if plugin.is_empty() { String::new() } else { alloc::format!(" (plugin {})", plugin) };
        crate::write_stderr(alloc::format!("PERMDENY {}{}{}{}\n", kind, sep, target, of).as_bytes());
    }
}

// ---------------------------------------------------------------- checks

// what a check asks for
enum Req<'a> {
    Fs(&'a [String], u32),
    ProcPath(Vec<String>),
    ProcName(String),
    Url(&'a str),
    Plugin(Vec<String>),
    Print,
    Secrets,
    Autostart,
    Audio,
    Hotkeys,
    Capture,
    #[cfg(any(windows, target_os = "linux"))]
    Connect([u8; 16], u16),
    #[cfg(any(windows, target_os = "linux"))]
    Listen([u8; 16], u16),
    Name(String),
    /// a path normalize refuses
    Never,
}

// a policy's answer to a request
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grant {
    No,
    Allow,
    Ask(u16),
}

// the ask tags of the matching rules -> a plain rule allows, else the first ask line asks
fn grant(tags: impl Iterator<Item = u16>) -> Grant {
    let mut g = Grant::No;
    for t in tags {
        if t == 0 {
            return Grant::Allow;
        }
        if g == Grant::No {
            g = Grant::Ask(t);
        }
    }
    g
}

fn flag(f: Option<u16>) -> Grant {
    match f {
        None => Grant::No,
        Some(0) => Grant::Allow,
        Some(k) => Grant::Ask(k),
    }
}

fn fs_match(r: &Rule, segs: &[String], mode: u32) -> bool {
    (r.mode & mode == mode || r.mode & FS_WRITE != 0) && seg_match(&r.segs, segs)
}

impl Policy {
    // granted: the user's files this policy may use; resolved: the app's resolved names
    // (own: their rules' ports; a plugin's: checked against its own name rules)
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(unused_variables))]
    fn grant(&self, req: &Req, granted: &[Rule], resolved: &[Resolved], own: bool) -> Grant {
        match req {
            Req::Fs(segs, mode) => grant(
                self.fs.iter().filter(|r| fs_match(r, segs, *mode)).map(|r| r.ask)
                    .chain(granted.iter().filter(|r| fs_match(r, segs, *mode)).map(|_| 0)),
            ),
            Req::ProcPath(segs) => grant(self.process.iter().filter(|(r, _)| seg_match(r, segs)).map(|r| r.1)),
            Req::ProcName(n) => grant(self.names.iter().filter(|(r, _)| r == n).map(|r| r.1)),
            Req::Url(s) if s.is_empty() => Grant::No,
            Req::Url(s) => grant(self.urls.iter().filter(|(u, _)| u == "*" || u == s).map(|u| u.1)),
            Req::Plugin(segs) => grant(self.plugins.iter().filter(|(r, _)| seg_match(r, segs)).map(|r| r.1)),
            Req::Print => flag(self.print),
            Req::Secrets => flag(self.secrets),
            Req::Autostart => flag(self.autostart),
            Req::Audio => flag(self.audio),
            Req::Hotkeys => flag(self.hotkeys),
            Req::Capture => flag(self.capture),
            #[cfg(any(windows, target_os = "linux"))]
            Req::Connect(ip, port) => {
                let dns = *port == 53 && self.connect.iter().any(|r| matches!(r.host, Host::Any | Host::Name(_)));
                let names = resolved.iter().filter(|a| a.ip == *ip).filter_map(|a| {
                    if own {
                        ((a.lo..=a.hi).contains(port)).then_some(a.ask)
                    } else {
                        self.connect.iter().any(|r| (r.lo..=r.hi).contains(port) && r.host.has_name(&a.name)).then_some(0)
                    }
                });
                grant(
                    self.connect.iter().filter(|r| (r.lo..=r.hi).contains(port) && r.host.has_ip(ip)).map(|r| r.ask)
                        .chain(names)
                        .chain(dns.then_some(0)),
                )
            }
            #[cfg(any(windows, target_os = "linux"))]
            Req::Listen(ip, port) => grant(self.listen.iter().filter(|r| (r.lo..=r.hi).contains(port) && r.host.has_ip(ip)).map(|r| r.ask)),
            Req::Name(h) => grant(self.connect.iter().filter(|r| r.host.has_name(h)).map(|r| r.ask)),
            Req::Never => Grant::No,
        }
    }
}

// the app's rules after the plugin scopes of this thread; asks when an ask line decides.
// fs: the Key::trusted bypass counts (outside plugin scopes)
fn decide(kind: &'static str, target: &str, req: &Req, fs: bool) -> Verdict {
    let scopes = scopes_here();
    let plugin = scopes.last().map_or(String::new(), |s| s.name.clone());
    if !scopes.is_empty() {
        let refused = {
            let g = POLICY.lock();
            let (granted, resolved): (&[Rule], &[Resolved]) = match g.as_ref() {
                Some(p) => (&p.granted, &p.resolved),
                None => (&[], &[]),
            };
            scopes.iter().find(|s| s.p.as_ref().is_some_and(|pp| pp.grant(req, if pp.user { granted } else { &[] }, resolved, false) != Grant::Allow)).map(|s| s.name.clone())
        };
        if let Some(name) = refused {
            record(kind, target, &name, true);
            return Verdict::Deny;
        }
    } else if fs && bypassed() {
        return Verdict::Allow;
    }
    if !active() {
        return Verdict::Allow;
    }
    let ask = {
        let g = POLICY.lock();
        let Some(p) = g.as_ref() else { return Verdict::Allow };
        match p.grant(req, &p.granted, &p.resolved, true) {
            Grant::Allow => return Verdict::Allow,
            Grant::No => None,
            Grant::Ask(k) => p.asks.get(k as usize - 1).cloned(),
        }
    };
    match ask {
        Some((rule, reason)) => ask_user(kind, target, &rule, &reason, &plugin),
        None => {
            record(kind, target, &plugin, true);
            Verdict::Deny
        }
    }
}

// ---------------------------------------------------------------- questions

const AUTO_NONE: u8 = 0xFE;
const AUTO_UNSET: u8 = 0xFF;
static QUESTIONS: SpinLock<Vec<Question>> = SpinLock::new(Vec::new());
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
// answers and withdrawals: waiting threads sleep on this word
static EPOCH: AtomicU32 = AtomicU32::new(0);
// the asking (UI) thread: thread id + 1, 0 = nobody asks
static ASKER: AtomicU64 = AtomicU64::new(0);
// Answer as u8, AUTO_NONE = ask, AUTO_UNSET = read jfgui_PERM_ANSWER first
static AUTO: AtomicU8 = AtomicU8::new(AUTO_UNSET);
// ms other threads wait; u32::MAX = read jfgui_PERM_TIMEOUT first
static WAIT: AtomicU32 = AtomicU32::new(u32::MAX);

fn bump() {
    EPOCH.fetch_add(1, Ordering::AcqRel);
    #[cfg(any(windows, target_os = "linux"))]
    crate::sync::wake_all(&EPOCH);
}

fn auto() -> Option<Answer> {
    let mut v = AUTO.load(Ordering::Acquire);
    if v == AUTO_UNSET {
        let a = crate::env("jfgui_PERM_ANSWER").and_then(|w| Answer::parse(w.trim()));
        v = a.map_or(AUTO_NONE, |a| a as u8);
        let _ = AUTO.compare_exchange(AUTO_UNSET, v, Ordering::AcqRel, Ordering::Acquire);
        v = AUTO.load(Ordering::Acquire);
    }
    [Answer::Once, Answer::Always, Answer::Deny, Answer::Never].into_iter().find(|a| *a as u8 == v)
}

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn wait_ms() -> u32 {
    let v = WAIT.load(Ordering::Acquire);
    if v != u32::MAX {
        return v;
    }
    let ms = crate::env("jfgui_PERM_TIMEOUT").and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(WAIT_MS).min(u32::MAX - 1);
    let _ = WAIT.compare_exchange(u32::MAX, ms, Ordering::AcqRel, Ordering::Acquire);
    WAIT.load(Ordering::Acquire)
}

/// the questions waiting for the user, oldest first
pub fn questions() -> Vec<Question> {
    QUESTIONS.lock().clone()
}

// the question of an ask line: the answer of this run or the stored one, a fixed answer, the
// asking thread (later), a wait for it (other threads), or a refusal when nobody asks
fn ask_user(kind: &'static str, target: &str, rule: &str, reason: &str, plugin: &str) -> Verdict {
    let refuse = || {
        record(kind, target, plugin, true);
        Verdict::Deny
    };
    if let Some(ok) = decided(rule) {
        return if ok { Verdict::Allow } else { refuse() };
    }
    if let Some(a) = auto() {
        decide_rule(rule, a, reason);
        return if a.allows() { Verdict::Allow } else { refuse() };
    }
    let asker = ASKER.load(Ordering::Acquire);
    if asker == 0 {
        return refuse(); // nobody to ask: headless runs, console programs
    }
    let (id, new) = {
        let mut g = QUESTIONS.lock();
        match g.iter().find(|q| q.rule == rule) {
            Some(q) => (q.id, false),
            None => {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let q = Question {
                    id,
                    kind,
                    target: String::from(target),
                    rule: String::from(rule),
                    reason: String::from(reason),
                    plugin: String::from(plugin),
                };
                g.push(q);
                (id, true)
            }
        }
    };
    if new {
        let sep = if target.is_empty() { "" } else { " " };
        crate::write_stderr(alloc::format!("PERMASK {}{}{} ({})\n", kind, sep, target, rule).as_bytes());
        crate::wake();
    }
    if tid() + 1 == asker {
        record(kind, target, plugin, false);
        return Verdict::Pending;
    }
    match wait_answer(id, rule) {
        Some(true) => Verdict::Allow,
        _ => refuse(),
    }
}

// another thread: wait until question id is answered (Some(allowed)), withdrawn or the time is
// up (None)
#[cfg(any(windows, target_os = "linux"))]
fn wait_answer(id: u32, rule: &str) -> Option<bool> {
    let deadline = crate::time_us().saturating_add(wait_ms() as u64 * 1000);
    loop {
        let e = EPOCH.load(Ordering::Acquire);
        if let Some(ok) = decided(rule) {
            return Some(ok);
        }
        if !QUESTIONS.lock().iter().any(|q| q.id == id) {
            return decided(rule);
        }
        let now = crate::time_us();
        if now >= deadline {
            return None;
        }
        let ms = ((deadline - now) / 1000).clamp(1, 1000) as u32;
        crate::sync::wait(&EPOCH, e, Some(ms));
    }
}

// the browser has one thread: it is the asking one
#[cfg(not(any(windows, target_os = "linux")))]
fn wait_answer(_id: u32, _rule: &str) -> Option<bool> {
    None
}

// ---------------------------------------------------------------- answers

#[derive(Default)]
struct Store {
    loaded: bool,
    /// None: not chosen yet (jfgui_PERM_STORE, else STORE); Some(None): memory only
    path: Option<Option<String>>,
    /// the OS file of the store, folded segments (refused to the app's writes)
    os: Option<Vec<String>>,
    stored: Vec<(String, bool)>,
    session: Vec<(String, bool)>,
    /// reasons of rules that are not ask lines (agent actions asked in this run)
    reasons: Vec<(String, String)>,
}

static STORE_S: SpinLock<Store> = SpinLock::new(Store {
    loaded: false,
    path: None,
    os: None,
    stored: Vec::new(),
    session: Vec::new(),
    reasons: Vec::new(),
});

fn store_path(s: &Store) -> Option<String> {
    match &s.path {
        Some(p) => p.clone(),
        None => match crate::env("jfgui_PERM_STORE") {
            Some(e) if e.is_empty() => None,
            Some(e) => Some(e),
            None => Some(String::from(STORE)),
        },
    }
}

// the OS file behind a store name (None: a mount without OS files, e.g. mem://)
fn store_os(path: &str) -> Option<String> {
    match crate::vfs::route(path) {
        crate::vfs::Route::Os => Some(String::from(path)),
        crate::vfs::Route::Vfs(p, rest) => p.os_path(&rest),
        crate::vfs::Route::Bad => None,
    }
}

fn store_read(path: &str) -> Option<Vec<u8>> {
    match crate::vfs::route(path) {
        crate::vfs::Route::Os => crate::read_file_raw(path),
        crate::vfs::Route::Vfs(p, rest) => match p.os_path(&rest) {
            Some(o) => crate::read_file_raw(&o),
            None => p.read(&rest),
        },
        crate::vfs::Route::Bad => None,
    }
}

fn store_write(path: &str, data: &[u8]) -> bool {
    let os = |o: &str| {
        if let Some(k) = o.rfind(['/', '\\']) {
            crate::mkdir_raw(&o[..k]);
        }
        crate::write_file_raw(o, data)
    };
    match crate::vfs::route(path) {
        crate::vfs::Route::Os => os(path),
        crate::vfs::Route::Vfs(p, rest) => match p.os_path(&rest) {
            Some(o) => os(&o),
            None => p.write(&rest, data),
        },
        crate::vfs::Route::Bad => false,
    }
}

// read the stored answers once (outside the lock: a mount may take its own locks)
fn load() {
    let path = {
        let s = STORE_S.lock();
        if s.loaded {
            return;
        }
        store_path(&s)
    };
    let data = path.as_deref().and_then(store_read).unwrap_or_default();
    let os = path.as_deref().and_then(store_os).and_then(|o| normalize(&o)).map(|n| split(&fold(&n)));
    let mut stored = Vec::new();
    for line in String::from_utf8_lossy(&data).lines() {
        let line = line.trim();
        let Some((w, rule)) = line.split_once(' ') else { continue };
        let rule = rule.split_whitespace().collect::<Vec<_>>().join(" ");
        let ok = match w {
            "always" => true,
            "never" => false,
            _ => continue,
        };
        if !rule.is_empty() && !kind_of(&rule).is_empty() && !stored.iter().any(|(r, _): &(String, bool)| *r == rule) {
            stored.push((rule, ok));
        }
    }
    let mut s = STORE_S.lock();
    if !s.loaded {
        s.loaded = true;
        s.stored = stored;
        s.os = os;
    }
}

// the answer of this run, else the stored one
fn decided(rule: &str) -> Option<bool> {
    load();
    let s = STORE_S.lock();
    s.session.iter().chain(s.stored.iter()).find(|(r, _)| r == rule).map(|x| x.1)
}

// write the stored answers (outside the lock)
fn save() -> bool {
    let (path, text) = {
        let s = STORE_S.lock();
        let Some(path) = store_path(&s) else { return true };
        let mut t = String::from("# the answers to the app's permission questions (docs/PERMISSIONS.md); delete a line to be asked again\n");
        for (r, ok) in &s.stored {
            t.push_str(if *ok { "always " } else { "never " });
            t.push_str(r);
            t.push('\n');
        }
        (path, t)
    };
    store_write(&path, text.as_bytes())
}

// the answer a for rule: this run (Once / Deny) or stored (Always / Never)
fn decide_rule(rule: &str, a: Answer, reason: &str) {
    load();
    {
        let mut s = STORE_S.lock();
        s.session.retain(|(r, _)| r != rule);
        s.stored.retain(|(r, _)| r != rule);
        if a.stored() {
            s.stored.push((String::from(rule), a.allows()));
        } else {
            s.session.push((String::from(rule), a.allows()));
        }
        if !reason.is_empty() && !s.reasons.iter().any(|(r, _)| r == rule) {
            s.reasons.push((String::from(rule), String::from(reason)));
        }
    }
    if a.stored() && !save() {
        crate::write_stderr(b"PERMDENY: the answer could not be stored\n");
    }
}

// whether segs is the store's OS file (refused to the app's writes)
fn store_protects(segs: &[String]) -> bool {
    load();
    STORE_S.lock().os.as_deref() == Some(segs)
}

/// every ask line of the manifest and every answered rule (agent actions, stored answers of
/// lines the manifest no longer has) with its answer, in that order
pub fn decisions() -> Vec<Decision> {
    load();
    let asks: Vec<(String, String)> = POLICY.lock().as_ref().map_or(Vec::new(), |p| p.asks.clone());
    let s = STORE_S.lock();
    let answer = |rule: &str| {
        if let Some((_, ok)) = s.session.iter().find(|(r, _)| r == rule) {
            return Some(if *ok { Answer::Once } else { Answer::Deny });
        }
        s.stored.iter().find(|(r, _)| r == rule).map(|(_, ok)| if *ok { Answer::Always } else { Answer::Never })
    };
    let mut out: Vec<Decision> = Vec::new();
    for (rule, reason) in &asks {
        if !out.iter().any(|d| d.rule == *rule) {
            out.push(Decision { rule: rule.clone(), kind: kind_of(rule), reason: reason.clone(), answer: answer(rule) });
        }
    }
    for (rule, _) in s.session.iter().chain(s.stored.iter()) {
        if !out.iter().any(|d| d.rule == *rule) {
            let reason = s.reasons.iter().find(|(r, _)| r == rule).map_or(String::new(), |x| x.1.clone());
            out.push(Decision { rule: rule.clone(), kind: kind_of(rule), reason, answer: answer(rule) });
        }
    }
    out
}

/// revoke the answer for rule (this run's and the stored one): the next access asks again ->
/// false when there was none
pub fn forget(rule: &str) -> bool {
    load();
    let (had, stored) = {
        let mut s = STORE_S.lock();
        let (n, m) = (s.session.len(), s.stored.len());
        s.session.retain(|(r, _)| r != rule);
        s.stored.retain(|(r, _)| r != rule);
        (s.session.len() != n || s.stored.len() != m, s.stored.len() != m)
    };
    if stored {
        save();
    }
    had
}

/// revoke every answer (this run's and the stored ones)
pub fn forget_all() {
    load();
    let stored = {
        let mut s = STORE_S.lock();
        s.session.clear();
        let had = !s.stored.is_empty();
        s.stored.clear();
        had
    };
    if stored {
        save();
    }
}

/// read the stored answers again (another program of the app changed them)
pub fn reload() {
    STORE_S.lock().loaded = false;
    load();
}

// ---------------------------------------------------------------- agent actions

/// agent.deny / agent.confirm of the manifest for action (Free without a manifest)
pub fn agent_rule(action: &str) -> AgentRule {
    let g = POLICY.lock();
    let Some(p) = g.as_ref() else { return AgentRule::Free };
    let m = |pats: &[String]| pats.iter().any(|x| glob(x.as_bytes(), action.as_bytes()));
    if m(&p.agent_deny) {
        AgentRule::Deny
    } else if m(&p.agent_confirm) {
        AgentRule::Confirm
    } else {
        AgentRule::Free
    }
}

/// ask the user whether an agent may run action (one decision per action, "agent ACTION");
/// description is shown as the reason
pub fn agent_ask(action: &str, description: &str) -> Verdict {
    ask_user("agent", action, &alloc::format!("agent {}", action), description, "")
}

/// record that an agent was refused action (agent.deny, an action the app keeps to itself)
pub fn agent_refused(action: &str) {
    record("agent", action, "", true);
}

// ---------------------------------------------------------------- plugins

/// the rights of a native plugin: its own manifest (None: unrestricted, the app has none and
/// neither has the plugin)
pub struct PluginRights {
    name: String,
    p: Option<Policy>,
}

impl PluginRights {
    /// a plugin's manifest text (the syntax of PERMISSIONS without ask and agent lines;
    /// $PLUGIN = dir); Err names the bad line
    pub fn parse(name: &str, text: &str, dir: &str) -> Result<Arc<PluginRights>, String> {
        let p = parse(text, &alloc::format!("{}.permissions", name), Some(dir))?;
        Ok(Arc::new(PluginRights { name: String::from(name), p: Some(p) }))
    }

    /// no rights at all (a plugin without a manifest in an app with one)
    pub fn none(name: &str) -> Arc<PluginRights> {
        Arc::new(PluginRights { name: String::from(name), p: Some(Policy::default()) })
    }

    /// no restriction of its own (the app's rules still apply)
    pub fn unrestricted(name: &str) -> Arc<PluginRights> {
        Arc::new(PluginRights { name: String::from(name), p: None })
    }

    /// the plugin's name (its file name without extension)
    pub fn name(&self) -> &str {
        &self.name
    }

    /// whether the plugin has rules of its own
    pub fn restricted(&self) -> bool {
        self.p.is_some()
    }
}

// threads inside on_behalf: (thread id, rights), innermost last
static SCOPES: SpinLock<Vec<(u64, Arc<PluginRights>)>> = SpinLock::new(Vec::new());
static SCOPED: AtomicUsize = AtomicUsize::new(0);

/// the rights of the native library at lib_path: its manifest "<stem>.permissions" beside it;
/// without one: no rights when the app has a manifest, else unrestricted.  Err: a manifest
/// that does not parse (do not load the plugin).
pub fn plugin_rights(lib_path: &str) -> Result<Arc<PluginRights>, String> {
    let k = lib_path.rfind(['/', '\\']).map_or(0, |k| k + 1);
    let (dir, file) = (&lib_path[..k.saturating_sub(1)], &lib_path[k..]);
    let stem = file.rfind('.').map_or(file, |d| &file[..d]);
    let man = alloc::format!("{}{}{}.permissions", dir, if dir.is_empty() { "" } else { "/" }, stem);
    match crate::read_file_raw(&man) {
        Some(b) => {
            let text = core::str::from_utf8(&b).map_err(|_| alloc::format!("{}.permissions: not UTF-8", stem))?;
            let d = if dir.is_empty() { crate::cwd() } else { String::from(dir) };
            PluginRights::parse(stem, text, &d)
        }
        None if active() => Ok(PluginRights::none(stem)),
        None => Ok(PluginRights::unrestricted(stem)),
    }
}

/// f on this thread on behalf of the plugin: every check must also pass its rules
pub fn on_behalf<R>(rights: &Arc<PluginRights>, f: impl FnOnce() -> R) -> R {
    if rights.p.is_none() {
        return f(); // unrestricted: no rules to add
    }
    let me = tid();
    SCOPES.lock().push((me, rights.clone()));
    SCOPED.fetch_add(1, Ordering::AcqRel);
    let r = f();
    {
        let mut g = SCOPES.lock();
        if let Some(i) = g.iter().rposition(|(t, s)| *t == me && Arc::ptr_eq(s, rights)) {
            g.remove(i);
        }
    }
    SCOPED.fetch_sub(1, Ordering::AcqRel);
    r
}

/// the innermost plugin scope of this thread
pub fn plugin_scope() -> Option<Arc<PluginRights>> {
    scopes_here().pop()
}

/// (task.rs, pool.rs) the plugin scopes of this thread, outermost first: jobs they start keep them
pub(crate) fn scopes_here() -> Vec<Arc<PluginRights>> {
    if SCOPED.load(Ordering::Acquire) == 0 {
        return Vec::new();
    }
    let me = tid();
    SCOPES.lock().iter().filter(|(t, _)| *t == me).map(|(_, s)| s.clone()).collect()
}

/// (task.rs, pool.rs) f inside the scopes (outermost first)
pub(crate) fn in_scopes<R>(scopes: &[Arc<PluginRights>], f: impl FnOnce() -> R) -> R {
    match scopes.split_first() {
        None => f(),
        Some((s, rest)) => on_behalf(s, || in_scopes(rest, f)),
    }
}

fn scoped_here() -> bool {
    SCOPED.load(Ordering::Acquire) != 0 && SCOPES.lock().iter().any(|(t, _)| *t == tid())
}

fn scope_name() -> String {
    if SCOPED.load(Ordering::Acquire) == 0 {
        return String::new();
    }
    let me = tid();
    SCOPES.lock().iter().rev().find(|(t, _)| *t == me).map_or(String::new(), |(_, s)| s.name.clone())
}

// ---------------------------------------------------------------- the manifest

// the keys an ask line may carry
const ASK_KEYS: [&str; 13] = [
    "fs.read", "fs.write", "process", "url", "plugins", "print", "secrets", "autostart", "audio.output", "net.connect",
    "net.listen", "input.global", "screen.capture",
];

// a line -> (rule text, reason): the reason is a "quoted text" at the end; '#' outside it
// starts a comment
fn split_line(line: &str) -> Result<(&str, Option<&str>), &'static str> {
    let cut = line.find(['#', '"']).unwrap_or(line.len());
    if line[cut..].starts_with('#') || cut == line.len() {
        return Ok((&line[..cut], None));
    }
    let rest = &line[cut + 1..];
    let Some(end) = rest.find('"') else { return Err("the reason has no closing quote") };
    let after = rest[end + 1..].trim_start();
    if !(after.is_empty() || after.starts_with('#')) {
        return Err("nothing but a comment may follow the reason");
    }
    Ok((&line[..cut], Some(&rest[..end])))
}

// src names the file in messages; plugin: the plugin's folder ($PLUGIN; no ask / agent lines,
// no implied app folders)
fn parse(text: &str, src: &str, plugin: Option<&str>) -> Result<Policy, String> {
    let mut p = Policy::default();
    match plugin {
        Some(dir) => add_dir_rule(&mut p.fs, dir, FS_READ),
        None => {
            for d in font_dirs() {
                add_dir_rule(&mut p.fs, &d, FS_READ);
            }
            for d in crate::spell::hunspell_dirs().into_iter().chain(crate::spell::hyphen_dirs()) {
                add_dir_rule(&mut p.fs, &d, FS_READ); // spell checking, hyphenation: the system's dictionaries
            }
            #[cfg(target_os = "linux")]
            for d in fontconfig_files() {
                add_dir_rule(&mut p.fs, &d, FS_READ); // fontconfig's aliases and language rules (fontconf.rs)
            }
            if let Some(e) = exe_dir() {
                add_dir_rule(&mut p.fs, &e, FS_READ);
            }
            for k in [crate::vfs::APP_CONFIG, crate::vfs::APP_DATA, crate::vfs::APP_CACHE, crate::vfs::APP_LOG] {
                if let Some(d) = crate::vfs::app_dir(k) {
                    add_dir_rule(&mut p.fs, &d, FS_WRITE);
                }
            }
            if cfg!(target_os = "linux") {
                for f in ["/etc/resolv.conf", "/etc/hosts"] {
                    p.fs.push(Rule { segs: split(f), mode: FS_READ, ask: 0 });
                }
            }
        }
    }
    for (i, line) in text.lines().enumerate() {
        let at = |msg: &str| alloc::format!("{}:{}: {}", src, i + 1, msg);
        let (rule, reason) = split_line(line).map_err(at)?;
        let mut words = rule.split_whitespace();
        let Some(mut key) = words.next() else {
            if reason.is_some() {
                return Err(at("a reason belongs to an ask line"));
            }
            continue;
        };
        let asking = key == "ask";
        if asking {
            if plugin.is_some() {
                return Err(at("a plugin cannot ask; the app's manifest asks"));
            }
            key = words.next().ok_or_else(|| at("ask needs a permission (ask fs.read $DOCUMENTS/**)"))?;
            if !ASK_KEYS.contains(&key) {
                return Err(at(&alloc::format!("{} cannot be asked for ({})", key, ASK_KEYS.join(", "))));
            }
        } else if reason.is_some() {
            return Err(at("a reason belongs to an ask line"));
        }
        let args: Vec<&str> = words.collect();
        if let Some(r) = reason {
            if r.len() > 200 || r.chars().any(|c| (c as u32) < 0x20) {
                return Err(at("a reason has at most 200 bytes and no control characters"));
            }
        }
        // the ask tag of this line's rules
        let ask = if asking {
            let mut text = String::from(key);
            for a in &args {
                text.push(' ');
                text.push_str(a);
            }
            p.asks.push((text, String::from(reason.unwrap_or(""))));
            if p.asks.len() > u16::MAX as usize {
                return Err(at("too many ask lines"));
            }
            p.asks.len() as u16
        } else {
            0
        };
        let pat = |a: &str| pattern(a, plugin).map_err(|e| at(&e));
        match key {
            "fs.read" | "fs.write" => {
                if args.is_empty() {
                    return Err(at("expected at least one pattern"));
                }
                let mode = if key == "fs.write" { FS_WRITE } else { FS_READ };
                for a in &args {
                    for segs in pat(a)? {
                        p.fs.push(Rule { segs, mode, ask });
                    }
                }
                if mode == FS_WRITE {
                    p.explicit_write = true;
                } else {
                    p.explicit_read = true;
                }
            }
            "fs.user" | "print" | "secrets" | "autostart" | "audio.output" | "input.global" | "screen.capture" => {
                if !args.is_empty() {
                    return Err(at("takes no arguments"));
                }
                // a plain line wins, else the first ask line
                let set = |f: &mut Option<u16>| *f = Some(f.map_or(ask, |o| o.min(ask)));
                match key {
                    "print" => set(&mut p.print),
                    "secrets" => set(&mut p.secrets),
                    "autostart" => set(&mut p.autostart),
                    "audio.output" => set(&mut p.audio),
                    "input.global" => set(&mut p.hotkeys),
                    "screen.capture" => set(&mut p.capture),
                    _ => p.user = true,
                }
            }
            "process" | "plugins" => {
                if args.is_empty() {
                    return Err(at("expected at least one pattern"));
                }
                for a in &args {
                    if key == "process" && !a.contains(['/', '\\', '$']) {
                        p.names.push((fold(a), ask));
                        continue;
                    }
                    for segs in pat(a)? {
                        if key == "process" {
                            p.process.push((segs, ask));
                        } else {
                            p.plugins.push((segs, ask));
                        }
                    }
                }
            }
            "net.connect" | "net.listen" => {
                if args.is_empty() {
                    return Err(at("expected at least one HOST:PORT"));
                }
                for a in &args {
                    let mut r = net_rule(a, key == "net.listen").map_err(|e| at(&e))?;
                    r.ask = ask;
                    if key == "net.listen" {
                        p.listen.push(r);
                    } else {
                        p.connect.push(r);
                    }
                }
            }
            "url" => {
                if args.is_empty() {
                    return Err(at("expected at least one scheme or *"));
                }
                for a in &args {
                    let ok = *a == "*" || a.bytes().all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(&c));
                    if !ok {
                        return Err(at("a scheme has only letters, digits, '+', '-' and '.'"));
                    }
                    p.urls.push((a.to_ascii_lowercase(), ask));
                }
            }
            "recent" => {
                if plugin.is_some() {
                    return Err(at("recent belongs to the app's manifest"));
                }
                if !args.is_empty() {
                    return Err(at("takes no arguments"));
                }
            }
            "sandbox" => {
                if plugin.is_some() {
                    return Err(at("sandbox belongs to the app's manifest"));
                }
                let lvl = match args.as_slice() {
                    [] => 1,
                    ["strict"] => 2,
                    _ => return Err(at("sandbox takes nothing or 'strict'")),
                };
                p.sandbox = p.sandbox.max(lvl);
            }
            "agent.confirm" | "agent.deny" => {
                if plugin.is_some() {
                    return Err(at("agent rules belong to the app's manifest"));
                }
                if args.is_empty() {
                    return Err(at("expected at least one action name (document.save, document.*, *)"));
                }
                for a in &args {
                    if a.len() > 128 || !a.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-*?".contains(&c)) {
                        return Err(at(&alloc::format!("{} is not an action name pattern (letters, digits, . _ - and * ?)", a)));
                    }
                    if key == "agent.deny" {
                        p.agent_deny.push(String::from(*a));
                    } else {
                        p.agent_confirm.push(String::from(*a));
                    }
                }
            }
            _ => {
                return Err(at(
                    "unknown permission (fs.read, fs.write, fs.user, process, url, plugins, print, secrets, autostart, audio.output, net.connect, net.listen, input.global, screen.capture, sandbox, ask, agent.confirm, agent.deny)",
                ))
            }
        }
    }
    Ok(p)
}

fn add_dir_rule(rules: &mut Vec<Rule>, dir: &str, mode: u32) {
    if let Some(n) = normalize(dir) {
        let mut segs = split(&fold(&n));
        segs.push(String::from("**"));
        rules.push(Rule { segs, mode, ask: 0 });
    }
}

// the variables of the user's folders, in DIR_* order (dirs.rs)
const USER_VARS: [&str; crate::DIR_COUNT as usize] = ["DESKTOP", "DOCUMENTS", "DOWNLOADS", "PICTURES", "MUSIC", "VIDEOS"];

// a pattern -> its segment lists (several for $FONTS, none for a variable without a value);
// plugin: the folder of $PLUGIN (a plugin's manifest)
fn pattern(a: &str, plugin: Option<&str>) -> Result<Vec<Vec<String>>, String> {
    if a == "**" {
        return Ok(alloc::vec![alloc::vec![String::from("**")]]);
    }
    let bases: Vec<String> = if let Some(v) = a.strip_prefix('$') {
        let end = v.find(['/', '\\']).unwrap_or(v.len());
        let base = match &v[..end] {
            "CONFIG" => crate::config_dir().into_iter().collect(),
            "TEMP" => crate::temp_dir().into_iter().collect(),
            "HOME" => crate::home().into_iter().collect(),
            "EXE" => exe_dir().into_iter().collect(),
            "FONTS" => font_dirs(),
            "APPCONFIG" => crate::vfs::app_dir(crate::vfs::APP_CONFIG).into_iter().collect(),
            "APPDATA" => crate::vfs::app_dir(crate::vfs::APP_DATA).into_iter().collect(),
            "APPCACHE" => crate::vfs::app_dir(crate::vfs::APP_CACHE).into_iter().collect(),
            "APPLOG" => crate::vfs::app_dir(crate::vfs::APP_LOG).into_iter().collect(),
            "PLUGIN" if plugin.is_some() => plugin.map(String::from).into_iter().collect(),
            "PLUGIN" => return Err(String::from("$PLUGIN only in a plugin's manifest")),
            other => match USER_VARS.iter().position(|u| *u == other) {
                Some(k) => crate::user_dir(k as u32).into_iter().collect(),
                None => {
                    return Err(alloc::format!(
                        "unknown variable ${} (CONFIG, TEMP, HOME, EXE, FONTS, APPCONFIG, APPDATA, APPCACHE, APPLOG, {}{})",
                        other,
                        USER_VARS.join(", "),
                        if plugin.is_some() { ", PLUGIN" } else { "" }
                    ));
                }
            },
        };
        base.into_iter().map(|b| alloc::format!("{}{}", b, &v[end..])).collect()
    } else {
        // "X:/.." and "//server/share/.." are Windows paths, "/.." Linux paths; the other
        // OS ignores them, so one manifest serves both
        let b = a.as_bytes();
        let win_abs = (is_drive(a) && matches!(b.get(2), Some(b'/' | b'\\'))) || a.starts_with("//") || a.starts_with("\\\\");
        let lin_abs = !win_abs && a.starts_with('/');
        if !win_abs && !lin_abs {
            return Err(alloc::format!("{} is not absolute (start it with /, X:/ or a variable)", a));
        }
        if win_abs != cfg!(windows) {
            return Ok(Vec::new());
        }
        alloc::vec![String::from(a)]
    };
    let mut out = Vec::new();
    for b in bases {
        match norm(&b, cfg!(windows), true, &|| String::new()) {
            Some(n) => out.push(split(&fold(&n))),
            None => return Err(alloc::format!("{} is not a valid path pattern", a)),
        }
    }
    Ok(out)
}

/// the system font folders in the order fontdb scans them: jfgui_FONTDIRS (';' separated,
/// on Linux also ':'), else the Windows / XDG folders; on Linux then fontconfig's own folders
/// (fontconfig_dirs: NixOS, TeX fonts, a <dir> of the user) and in Flatpak the host's fonts
pub fn font_dirs() -> Vec<String> {
    let mut v = Vec::new();
    if let Some(e) = crate::env("jfgui_FONTDIRS").filter(|e| !e.is_empty()) {
        let sep = |c: char| c == ';' || (cfg!(target_os = "linux") && c == ':');
        return e.split(sep).filter(|s| !s.is_empty()).map(String::from).collect();
    }
    let env = |var: &str, suffix: &str| crate::env(var).filter(|s| !s.is_empty()).map(|s| s + suffix);
    if cfg!(windows) {
        v.extend(env("WINDIR", "/Fonts"));
        v.extend(env("LOCALAPPDATA", "/Microsoft/Windows/Fonts"));
    } else {
        v.push(String::from("/usr/share/fonts"));
        v.push(String::from("/usr/local/share/fonts"));
        match env("XDG_DATA_HOME", "/fonts") {
            Some(d) => v.push(d),
            None => v.extend(env("HOME", "/.local/share/fonts")),
        }
        v.extend(env("HOME", "/.fonts"));
        #[cfg(target_os = "linux")]
        for d in fontconfig_dirs() {
            // a folder inside one already listed is scanned with it
            if !v.iter().any(|x| d == *x || d.strip_prefix(x.as_str()).is_some_and(|r| r.starts_with('/'))) {
                v.push(d);
            }
        }
    }
    v
}

/// (Linux) fontconfig's files the font code reads (fontconf.rs): its folder and the user's
#[cfg(target_os = "linux")]
fn fontconfig_files() -> Vec<String> {
    let mut v = alloc::vec![String::from("/etc/fonts")];
    match crate::env("XDG_CONFIG_HOME").filter(|s| s.starts_with('/')) {
        Some(c) => v.push(alloc::format!("{}/fontconfig", c.trim_end_matches('/'))),
        None => v.extend(crate::env("HOME").filter(|s| !s.is_empty()).map(|h| alloc::format!("{}/.config/fontconfig", h.trim_end_matches('/')))),
    }
    v
}

/// (Linux) the font folders fontconfig's files name (<dir>, <remap-dir>) in /etc/fonts/fonts.conf,
/// /etc/fonts/conf.d and the user's fontconfig folder -- NixOS and Guix have no /usr/share/fonts,
/// Debian's TeX fonts stay in /usr/share/texmf -- and in Flatpak the host's fonts the runtime
/// mounts under /run/host (font-dirs.xml).  A light reading: comments dropped, prefix="xdg"
/// ($XDG_DATA_HOME, else ~/.local/share), "~/", absolute paths; at most 128 files of 256 KiB
#[cfg(target_os = "linux")]
fn fontconfig_dirs() -> Vec<String> {
    let confd = |files: &mut Vec<String>, dir: &str| {
        let mut names = Vec::new();
        crate::dir_list_raw(dir, &mut |n, is_dir, _| {
            if !is_dir && n.ends_with(".conf") {
                names.push(alloc::format!("{}/{}", dir, n));
            }
        });
        names.sort();
        files.extend(names);
    };
    let mut files: Vec<String> = alloc::vec![String::from("/etc/fonts/fonts.conf")];
    confd(&mut files, "/etc/fonts/conf.d");
    for c in fontconfig_files().into_iter().skip(1) {
        files.push(alloc::format!("{}/fonts.conf", c));
        confd(&mut files, &alloc::format!("{}/conf.d", c));
    }
    if crate::file_exists_raw("/.flatpak-info") == 1 {
        files.push(String::from("/run/host/font-dirs.xml"));
    }
    let home = crate::env("HOME").filter(|h| h.starts_with('/'));
    let data_home = crate::env("XDG_DATA_HOME").filter(|d| d.starts_with('/')).or_else(|| home.as_ref().map(|h| alloc::format!("{}/.local/share", h.trim_end_matches('/'))));
    let mut out: Vec<String> = Vec::new();
    for f in files.iter().take(128) {
        let Some(data) = crate::read_file_raw(f) else { continue };
        if data.len() > 256 << 10 {
            continue;
        }
        let text = String::from_utf8_lossy(&data);
        // without comments
        let mut t = String::new();
        let mut rest: &str = &text;
        while let Some(a) = rest.find("<!--") {
            t.push_str(&rest[..a]);
            rest = rest[a..].find("-->").map_or("", |b| &rest[a + b + 3..]);
        }
        t.push_str(rest);
        for tag in ["dir", "remap-dir"] {
            let open = alloc::format!("<{}", tag);
            let close = alloc::format!("</{}>", tag);
            let mut at = 0;
            while let Some(k) = t[at..].find(&open) {
                let start = at + k + open.len();
                at = start;
                // <dir> or <dir attr...>, not <dirname>
                if !t[start..].starts_with(['>', ' ', '\t', '\n', '\r']) {
                    continue;
                }
                let Some(gt) = t[start..].find('>') else { break };
                let attrs = &t[start..start + gt];
                if attrs.ends_with('/') {
                    continue;
                }
                let body = start + gt + 1;
                let Some(end) = t[body..].find(&close) else { break };
                let p = t[body..body + end].trim().replace("&amp;", "&");
                at = body + end;
                let path = if attrs.contains("prefix=\"xdg\"") {
                    match &data_home {
                        Some(d) => alloc::format!("{}/{}", d, p),
                        None => continue,
                    }
                } else if let Some(r) = p.strip_prefix("~/") {
                    match &home {
                        Some(h) => alloc::format!("{}/{}", h.trim_end_matches('/'), r),
                        None => continue,
                    }
                } else if p.starts_with('/') {
                    p
                } else {
                    continue;
                };
                let path = String::from(path.trim_end_matches('/'));
                if !path.is_empty() && !out.contains(&path) {
                    out.push(path);
                }
            }
        }
    }
    out
}

#[cfg(any(windows, target_os = "linux"))]
fn exe_dir() -> Option<String> {
    let p = crate::current_exe()?;
    let k = p.rfind(['/', '\\'])?;
    Some(String::from(&p[..k]))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn exe_dir() -> Option<String> {
    None
}

// ---------------------------------------------------------------- the network

/// the host of a net.connect / net.listen rule (the browser keeps the rules unused)
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum Host {
    Any,
    /// a DNS name in lower case; "*.example.org": every name below example.org
    Name(String),
    /// an address block: 16 bytes (IPv4 as ::ffff:a.b.c.d) and the prefix bits
    Block([u8; 16], u8),
    /// 127.0.0.0/8 and ::1
    Loopback,
}

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct NetRule {
    host: Host,
    lo: u16,
    hi: u16,
    ask: u16,
}

#[cfg(any(windows, target_os = "linux"))]
const RESOLVED_MAX: usize = 4096;
#[cfg(any(windows, target_os = "linux"))]
const V6_LOOPBACK: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

impl Host {
    fn has_name(&self, h: &str) -> bool {
        match self {
            Host::Any => true,
            Host::Name(n) => match n.strip_prefix("*.") {
                Some(base) => h.len() > base.len() + 1 && h.ends_with(base) && h.as_bytes()[h.len() - base.len() - 1] == b'.',
                None => h == n,
            },
            _ => false,
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn has_ip(&self, ip: &[u8; 16]) -> bool {
        match self {
            Host::Any => true,
            Host::Name(_) => false,
            Host::Block(net, bits) => prefix_eq(ip, net, *bits),
            Host::Loopback => prefix_eq(ip, &mapped([127, 0, 0, 0]), 104) || *ip == V6_LOOPBACK,
        }
    }
}

fn mapped(v4: [u8; 4]) -> [u8; 16] {
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, v4[0], v4[1], v4[2], v4[3]]
}

#[cfg(any(windows, target_os = "linux"))]
fn prefix_eq(a: &[u8; 16], b: &[u8; 16], bits: u8) -> bool {
    let (whole, rest) = ((bits / 8) as usize, bits % 8);
    if a[..whole] != b[..whole] {
        return false;
    }
    rest == 0 || (a[whole] ^ b[whole]) & (0xFFu8 << (8 - rest)) == 0
}

// "HOST:PORT" of the manifest
fn net_rule(a: &str, listen: bool) -> Result<NetRule, String> {
    let err = || alloc::format!("{} is not HOST:PORT (example.org:443, *.example.org:*, 10.0.0.0/8:22, [::1]:8080, *:*)", a);
    let k = a.rfind(':').ok_or_else(err)?;
    let (host, port) = (&a[..k], &a[k + 1..]);
    let num = |s: &str| if !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()) { s.parse::<u16>().ok() } else { None };
    let (lo, hi) = if port == "*" {
        (0, u16::MAX)
    } else if let Some((x, y)) = port.split_once('-') {
        match (num(x), num(y)) {
            (Some(x), Some(y)) if x <= y => (x, y),
            _ => return Err(err()),
        }
    } else {
        let n = num(port).ok_or_else(err)?;
        (n, n)
    };
    let host = if host == "*" {
        Host::Any
    } else if host.eq_ignore_ascii_case("localhost") {
        Host::Loopback
    } else {
        let (h, bits) = match host.rsplit_once('/') {
            Some((h, b)) if !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()) => (h, Some(b.parse::<u32>().map_err(|_| err())?)),
            Some(_) => return Err(err()),
            None => (host, None),
        };
        let bracketed = h.len() >= 2 && h.starts_with('[') && h.ends_with(']');
        let bare = if bracketed { &h[1..h.len() - 1] } else { h };
        if !bracketed && bare.contains(':') {
            return Err(alloc::format!("{}: an IPv6 address needs brackets ([::1]:80)", a));
        }
        match ip16(bare) {
            Some((ip, v4)) => {
                let max = if v4 { 32 } else { 128 };
                let b = bits.unwrap_or(max);
                if b > max || (v4 && bracketed) {
                    return Err(err());
                }
                Host::Block(ip, (b + if v4 { 96 } else { 0 }) as u8)
            }
            None if bracketed || bits.is_some() => return Err(err()),
            None if listen => return Err(alloc::format!("{}: net.listen takes an address, a block, localhost or *", a)),
            None => {
                let n = bare.trim_end_matches('.').to_ascii_lowercase();
                let labels = n.strip_prefix("*.").unwrap_or(&n);
                let ok = !labels.is_empty()
                    && labels.split('.').all(|l| !l.is_empty() && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
                if !ok {
                    return Err(err());
                }
                Host::Name(n)
            }
        }
    };
    Ok(NetRule { host, lo, hi, ask: 0 })
}

// an IPv4 / IPv6 address -> 16 bytes, is IPv4
#[cfg(any(windows, target_os = "linux"))]
fn ip16(s: &str) -> Option<([u8; 16], bool)> {
    match crate::net::IpAddr::parse(s)? {
        crate::net::IpAddr::V4(a) => Some((mapped(a), true)),
        crate::net::IpAddr::V6(a) => Some((a, false)),
    }
}

// the browser has no sockets: a dotted IPv4 address (numeric labels) still parses, the rules
// are never asked
#[cfg(not(any(windows, target_os = "linux")))]
fn ip16(s: &str) -> Option<([u8; 16], bool)> {
    let p: Vec<&str> = s.split('.').collect();
    let v: Vec<u8> = p.iter().filter_map(|x| x.parse::<u8>().ok()).collect();
    if p.len() == 4 && v.len() == 4 { Some((mapped([v[0], v[1], v[2], v[3]]), true)) } else if s.contains(':') { Some(([0; 16], false)) } else { None }
}

#[cfg(any(windows, target_os = "linux"))]
fn to16(ip: &crate::net::IpAddr) -> [u8; 16] {
    match ip {
        crate::net::IpAddr::V4(a) => mapped(*a),
        crate::net::IpAddr::V6(a) => *a,
    }
}

/// whether a TCP connection or a UDP datagram to addr is allowed: a net.connect rule with
/// that address, an address a permitted name resolved to, or port 53 (DNS) with name rules
#[cfg(any(windows, target_os = "linux"))]
pub fn net_connect_allowed(addr: &crate::net::SocketAddr) -> bool {
    decide("net.connect", &alloc::format!("{}", addr), &Req::Connect(to16(&addr.ip), addr.port), false) == Verdict::Allow
}

/// whether a TCP listener (or a UDP socket with a fixed port) may bind addr
#[cfg(any(windows, target_os = "linux"))]
pub fn net_listen_allowed(addr: &crate::net::SocketAddr) -> bool {
    decide("net.listen", &alloc::format!("{}", addr), &Req::Listen(to16(&addr.ip), addr.port), false) == Verdict::Allow
}

/// whether host may be resolved: a net.connect rule names it (or allows every host)
pub fn net_name_allowed(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    decide("net.connect", host, &Req::Name(h), false) == Verdict::Allow
}

/// (resolvers) host resolved to addrs: connections to them get the ports of host's rules
#[cfg(any(windows, target_os = "linux"))]
pub fn net_resolved(host: &str, addrs: &[crate::net::SocketAddr]) {
    if !active() && SCOPED.load(Ordering::Acquire) == 0 {
        return;
    }
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    let mut g = POLICY.lock();
    let Some(p) = g.as_mut() else { return };
    let ranges: Vec<(u16, u16, u16)> =
        p.connect.iter().filter(|r| matches!(r.host, Host::Name(_) | Host::Any) && r.host.has_name(&h)).map(|r| (r.lo, r.hi, r.ask)).collect();
    for a in addrs {
        let ip = to16(&a.ip);
        for &(lo, hi, ask) in &ranges {
            if !p.resolved.iter().any(|x| x.ip == ip && x.lo == lo && x.hi == hi && x.ask == ask && x.name == h) {
                if p.resolved.len() == RESOLVED_MAX {
                    p.resolved.remove(0);
                }
                p.resolved.push(Resolved { ip, lo, hi, ask, name: h.clone() });
            }
        }
    }
}

// ---------------------------------------------------------------- paths

/// path as an absolute, normalised path of this OS (relative paths against cwd()), or None
/// for a path the policy refuses. The normalisation is lexical: junctions, symbolic links and
/// 8.3 names are not resolved, so rules match the path as written, not its final target.
pub fn normalize(path: &str) -> Option<String> {
    norm(path, cfg!(windows), false, &crate::cwd)
}

/// normalize with the rules of Windows (windows = true) or Linux against the folder cwd
pub fn normalize_for(path: &str, windows: bool, cwd: &str) -> Option<String> {
    norm(path, windows, false, &|| String::from(cwd))
}

// glob: '*' and '?' are pattern characters (manifest), not refused names
fn norm(path: &str, win: bool, glob: bool, cwd: &dyn Fn() -> String) -> Option<String> {
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    if !win {
        let full = if path.starts_with('/') {
            String::from(path)
        } else {
            let c = cwd();
            if !c.starts_with('/') {
                return None;
            }
            alloc::format!("{}/{}", c, path)
        };
        if full.split('/').any(|s| s == "..") {
            return None; // resolved after symbolic links by the kernel, not lexically
        }
        let segs = segments(&full, false, glob)?;
        return Some(alloc::format!("/{}", segs.join("/")));
    }
    let mut s: String = path.replace('\\', "/");
    // \\?\ and \\.\ prefixes: only drive and UNC paths
    for pre in ["//?/", "//./", "/??/"] {
        if let Some(rest) = s.strip_prefix(pre) {
            if rest.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("UNC/")) {
                s = alloc::format!("//{}", &rest[4..]);
            } else if is_drive(rest) {
                s = String::from(rest);
            } else {
                return None;
            }
            break;
        }
    }
    let (root, rest) = if let Some(unc) = s.strip_prefix("//") {
        let mut it = unc.splitn(3, '/');
        let server = it.next().filter(|x| !x.is_empty())?;
        let share = it.next().filter(|x| !x.is_empty())?;
        if !name_ok(server, glob) || !name_ok(share, glob) {
            return None;
        }
        (alloc::format!("//{}/{}", server, share), String::from(it.next().unwrap_or("")))
    } else if is_drive(&s) {
        let d = s.as_bytes()[0].to_ascii_uppercase() as char;
        match s.as_bytes().get(2) {
            None => (alloc::format!("{}:", d), String::new()),
            Some(b'/') => (alloc::format!("{}:", d), String::from(&s[3..])),
            Some(_) => return None, // "C:file": relative to the drive's own current folder
        }
    } else {
        let c = cwd();
        if c.is_empty() || !(c.starts_with(['/', '\\']) || is_drive(&c)) {
            return None;
        }
        let base = norm(&c, true, false, &|| String::new())?;
        if s.starts_with('/') {
            // rooted: the drive (or share) of the current folder
            let root = match base.strip_prefix("//") {
                Some(u) => {
                    let mut it = u.splitn(3, '/');
                    alloc::format!("//{}/{}", it.next()?, it.next()?)
                }
                None => String::from(&base[..2]),
            };
            return norm(&alloc::format!("{}{}", root, s), true, glob, cwd);
        }
        return norm(&alloc::format!("{}/{}", base.trim_end_matches('/'), s), true, glob, cwd);
    };
    let segs = segments(&rest, true, glob)?;
    if segs.is_empty() {
        return Some(if root.starts_with("//") { root } else { alloc::format!("{}/", root) });
    }
    Some(alloc::format!("{}/{}", root, segs.join("/")))
}

// "X:" at the start (absolute "X:/..", or the drive-relative "X:file" that norm refuses)
fn is_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

// "." / "" dropped, ".." removes the previous segment (None above the root); Windows: the
// trailing dots and blanks Win32 strips, no stream, device or forbidden names
fn segments(rest: &str, win: bool, glob: bool) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            _ if win => {
                let t = seg.trim_end_matches(['.', ' ']);
                if t.is_empty() || !name_ok(t, glob) {
                    return None;
                }
                out.push(String::from(t));
            }
            _ => out.push(String::from(seg)),
        }
    }
    Some(out)
}

// a Windows file name: no control characters, no < > : " | (and * ? outside patterns), no
// device name (CON, NUL, COM1 .., also with an extension)
fn name_ok(name: &str, glob: bool) -> bool {
    if name.chars().any(|c| (c as u32) < 0x20 || "<>:\"|".contains(c) || (!glob && "*?".contains(c))) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let u = stem.to_ascii_uppercase();
    let dev = matches!(u.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (u.len() == 4 && (u.starts_with("COM") || u.starts_with("LPT")) && u.as_bytes()[3].is_ascii_digit())
        || (u.len() == 5 && (u.starts_with("COM") || u.starts_with("LPT")) && ["¹", "²", "³"].contains(&&stem[3..]));
    !dev
}

// the form rules and paths are compared in: Windows without case
fn fold(s: &str) -> String {
    if cfg!(windows) { s.chars().flat_map(char::to_lowercase).collect() } else { String::from(s) }
}

fn split(n: &str) -> Vec<String> {
    let mut v: Vec<String> = n.split('/').map(String::from).collect();
    if v.len() > 1 && v.last().is_some_and(|s| s.is_empty()) {
        v.pop(); // "C:/" -> ["c:"]
    }
    v
}

// pattern segments against path segments ("**" = any number of segments)
fn seg_match(p: &[String], s: &[String]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some(f) if f == "**" => (0..=s.len()).any(|k| seg_match(&p[1..], &s[k..])),
        Some(f) => !s.is_empty() && glob(f.as_bytes(), s[0].as_bytes()) && seg_match(&p[1..], &s[1..]),
    }
}

// '*' any run, '?' one character, inside one segment (bytes; '?' takes one UTF-8 byte)
fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() && p[pi] == b'*' {
            pi += 1;
            star = Some((pi, si));
        } else if pi < p.len() && (p[pi] == b'?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if let Some((sp, ss)) = star {
            pi = sp;
            si = ss + 1;
            star = Some((sp, ss + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// whether pattern (manifest syntax after variables) matches path; both normalised with the
/// rules of this OS (tests)
pub fn pattern_matches(pattern: &str, path: &str) -> bool {
    let Ok(pats) = self::pattern(pattern, None) else { return false };
    let Some(n) = normalize(path) else { return false };
    let segs = split(&fold(&n));
    pats.iter().any(|p| seg_match(p, &segs))
}

/// (build checks, tests) whether text is a valid manifest: Err names the first bad line
pub fn check(text: &str) -> Result<(), String> {
    parse(text, "PERMISSIONS", None).map(|_| ())
}
