// instance -- one running instance per app and user (ui: AppDesc AF_SINGLE_INSTANCE): a later
// start hands its working folder and arguments to the running instance and ends
//
//   claim(app) -> Claim        Primary: this process is the app's instance; the requests of
//                              later starts arrive on a thread of their own (take(); each one
//                              calls wake()); Forwarded: the running instance has the request,
//                              end this process; Alone: no instance channel (browser, Android,
//                              an error): carry on alone
//   take() -> Vec<Request>     the requests that arrived, oldest first
//   send(app, cwd, args) -> bool    hand a request to the running instance (claim uses it)
//   Request { cwd, args }; Request::paths()   the arguments that are no options, as absolute
//                              paths (local file:// URIs as paths, other names with a scheme
//                              and URLs as they are)
//
// Channel (control_ipc_win.rs / control_ipc_linux.rs): Windows the pipe
// \\.\pipe\jfgui-instance-<user SID>-<app> (current user, local clients, one pipe instance:
// the first process that creates it is the instance; the starting process lets it take the
// foreground); Linux the abstract socket jfgui-instance-<uid>-<app>.  Both ends make sure the
// other runs as the same user.  Message: "JFI1", u32 length of the rest, u32 count, count x
// (u32 length, UTF-8): the working folder, then the arguments (without the program), on Linux
// then "XAT1", u32 length, the start's $XDG_ACTIVATION_TOKEN when it has one (the running
// instance's window comes forward with it on Wayland: linux_wl_act.rs; an instance of an older
// version refuses the block, then the request goes again without it); answer "OK".  At most
// 1 MiB, 3 s per exchange.

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

/// what claim found
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// this process is the app's instance
    Primary,
    /// the running instance got this start's request: end the process
    Forwarded,
    /// no instance channel here: carry on alone
    Alone,
}

/// the command line of a later start
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// the working folder of the starting process
    pub cwd: String,
    /// its arguments, without the program
    pub args: Vec<String>,
}

const MAGIC: &[u8; 4] = b"JFI1";
const TOKEN_TAG: &[u8; 4] = b"XAT1";
const MSG_MAX: usize = 1 << 20;
#[cfg(any(windows, target_os = "linux"))]
const QUEUE_MAX: usize = 64;

static QUEUE: SpinLock<Vec<Request>> = SpinLock::new(Vec::new());

impl Request {
    /// the arguments that are no options ("-..."): relative paths against cwd; local file://
    /// URIs as paths (Linux desktops hand files over that way for %U); other names with a
    /// scheme ("config://..", "https://..") as they are
    pub fn paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        for a in self.args.iter().filter(|a| !a.is_empty() && !a.starts_with('-')) {
            if let Some(p) = crate::assoc::file_uri_path(a) {
                out.push(p);
                continue;
            }
            let b = a.as_bytes();
            let absolute = a.contains("://")
                || b[0] == b'/'
                || b[0] == b'\\'
                || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':');
            if absolute || self.cwd.is_empty() {
                out.push(a.clone());
            } else {
                out.push(alloc::format!("{}/{}", self.cwd.trim_end_matches(['/', '\\']), a));
            }
        }
        out
    }
}

/// the requests that arrived since the last call, oldest first
pub fn take() -> Vec<Request> {
    core::mem::take(&mut *QUEUE.lock())
}

fn encode(cwd: &str, args: &[String], token: Option<&str>) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    body.extend_from_slice(&(args.len() as u32 + 1).to_le_bytes());
    for s in core::iter::once(cwd).chain(args.iter().map(|a| a.as_str())) {
        body.extend_from_slice(&u32::try_from(s.len()).ok()?.to_le_bytes());
        body.extend_from_slice(s.as_bytes());
    }
    if let Some(t) = token {
        body.extend_from_slice(TOKEN_TAG);
        body.extend_from_slice(&u32::try_from(t.len()).ok()?.to_le_bytes());
        body.extend_from_slice(t.as_bytes());
    }
    if body.len() > MSG_MAX {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

// -> the request and the start's activation token
fn decode(body: &[u8]) -> Option<(Request, Option<String>)> {
    let rd = |p: usize| -> Option<usize> { Some(u32::from_le_bytes(body.get(p..p + 4)?.try_into().ok()?) as usize) };
    let count = rd(0)?;
    let mut p = 4;
    let mut items = Vec::new();
    for _ in 0..count {
        let n = rd(p)?;
        let s = core::str::from_utf8(body.get(p + 4..p.checked_add(4 + n)?)?).ok()?;
        items.push(String::from(s));
        p += 4 + n;
    }
    let mut token = None;
    if body.get(p..p + 4) == Some(&TOKEN_TAG[..]) {
        let n = rd(p + 4)?;
        token = Some(String::from(core::str::from_utf8(body.get(p + 8..p.checked_add(8 + n)?)?).ok()?));
        p += 8 + n;
    }
    if p != body.len() || items.is_empty() {
        return None;
    }
    let cwd = items.remove(0);
    Some((Request { cwd, args: items }, token))
}

// ---------------------------------------------------------------- native

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::control_ipc::{InstConn, inst_connect, inst_listen};

    const STEP_MS: u32 = 2;
    // never set: the serving thread runs until the process ends
    static STOP: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    const LIMIT_MS: u64 = 3000;

    fn deadline() -> u64 {
        crate::time_us() + LIMIT_MS * 1000
    }

    fn write_all(c: &mut InstConn, mut data: &[u8], end: u64) -> bool {
        while !data.is_empty() {
            match c.write(data) {
                Ok(Some(n)) if n > 0 => data = &data[n..],
                Ok(Some(_)) | Err(_) => return false,
                Ok(None) if crate::time_us() < end => crate::sleep_ms(STEP_MS),
                Ok(None) => return false,
            }
        }
        true
    }

    fn read_exact(c: &mut InstConn, buf: &mut [u8], end: u64) -> bool {
        let mut got = 0;
        while got < buf.len() {
            match c.read(&mut buf[got..]) {
                Ok(Some(n)) if n > 0 => got += n,
                Ok(Some(_)) | Err(_) => return false,
                Ok(None) if crate::time_us() < end => crate::sleep_ms(STEP_MS),
                Ok(None) => return false,
            }
        }
        true
    }

    // one request from a starting process -> queued, answered
    fn serve_one(c: &mut InstConn) -> bool {
        let end = deadline();
        let mut head = [0u8; 8];
        if !read_exact(c, &mut head, end) || &head[..4] != MAGIC {
            return false;
        }
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if len > MSG_MAX {
            return false;
        }
        let mut body = alloc::vec![0u8; len];
        if !read_exact(c, &mut body, end) {
            return false;
        }
        let Some((req, _token)) = decode(&body) else { return false };
        #[cfg(target_os = "linux")]
        if let Some(t) = &_token {
            crate::act_provide(t); // before the UI brings its window forward
        }
        {
            let mut q = QUEUE.lock();
            if q.len() == QUEUE_MAX {
                q.remove(0);
            }
            q.push(req);
        }
        let _ = write_all(c, b"OK", end);
        crate::wake();
        true
    }

    pub fn claim(app: &str) -> Claim {
        if !app_ok(app) {
            return Claim::Alone;
        }
        for _ in 0..2 {
            match inst_listen(app) {
                Ok(Some(server)) => {
                    // the instance's lifetime: the thread ends with the process
                    let started = crate::thread_spawn(64 << 10, alloc::boxed::Box::new(move || {
                        while !STOP.load(core::sync::atomic::Ordering::Relaxed) {
                            match server.accept() {
                                Ok(mut c) => {
                                    serve_one(&mut c);
                                }
                                Err(_) => crate::sleep_ms(100),
                            }
                        }
                    }));
                    return if started.is_some() { Claim::Primary } else { Claim::Alone };
                }
                Ok(None) => {
                    let args: Vec<String> = crate::args().into_iter().skip(1).collect();
                    let token = if cfg!(target_os = "linux") { crate::env("XDG_ACTIVATION_TOKEN") } else { None };
                    let sent = |t: Option<&str>| encode(&crate::cwd(), &args, t).is_some_and(|m| send_msg(app, &m));
                    // (an older instance refuses the token's block: again without it)
                    if sent(token.as_deref()) || token.is_some() && sent(None) {
                        return Claim::Forwarded;
                    }
                    // the instance just ended (or does not answer): try to become it
                }
                Err(_) => return Claim::Alone,
            }
        }
        Claim::Alone
    }

    pub fn send(app: &str, cwd: &str, args: &[String]) -> bool {
        encode(cwd, args, None).is_some_and(|m| send_msg(app, &m))
    }

    fn send_msg(app: &str, msg: &[u8]) -> bool {
        if !app_ok(app) {
            return false;
        }
        let Ok(mut c) = inst_connect(app) else { return false };
        let end = deadline();
        let mut ok = [0u8; 2];
        write_all(&mut c, &msg, end) && read_exact(&mut c, &mut ok, end) && &ok == b"OK"
    }
}

// names of the channel: the app id rules of vfs::set_app_id
fn app_ok(app: &str) -> bool {
    !app.is_empty() && app.len() <= 64 && app.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

/// become the app's single instance, or hand this start's arguments to the running one
pub fn claim(app: &str) -> Claim {
    #[cfg(any(windows, target_os = "linux"))]
    {
        native::claim(app)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (app, app_ok as fn(&str) -> bool);
        Claim::Alone
    }
}

/// hand (cwd, args) to the running instance of app -> it confirmed the request
pub fn send(app: &str, cwd: &str, args: &[String]) -> bool {
    #[cfg(any(windows, target_os = "linux"))]
    {
        native::send(app, cwd, args)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (app, cwd, args, encode as fn(&str, &[String]) -> Option<Vec<u8>>, decode as fn(&[u8]) -> Option<Request>);
        false
    }
}
