// live -- the connection to a BackupCraft server over HTTP(S): worker threads send the queued
// requests (netio::Client: timeouts, size limit) and hand the answers back; plat::wake() makes
// the console take them.  A network failure answers status 0 ("no connection") and marks the
// link lost; the next success marks it online again.
//
// Addresses: "backup-srv01" / "backup-srv01:7642" / "https://..." -- without a scheme loopback
// names use http, everything else https; the port defaults to 7642.
//
// https: the server has its own CA.  The console trusts a CA file (--ca), or the fingerprint
// pinned when the user trusted the server, or asks: an unknown CA ends the request with the code
// "untrusted" and Live::presented() tells which CA it was (netio::trust).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use bc_proto::api::Method;
use bc_proto::j::{self, Value};
use netio::trust::{self, Presented, ServerTrust};
use netio::{Client, HttpError, Url};
use plat::sync::{channel, Mutex, Receiver, Sender};

use crate::net::{Link, Response};

/// requests in flight at once (polls must not wait behind a slow answer)
const WORKERS: usize = 3;

/// requests sent whose answers the console has not taken yet (scripts' `sync` waits for 0)
static PENDING: AtomicU32 = AtomicU32::new(0);

/// the sync hook of the UI: answers still outstanding
pub fn pending() -> bool {
    PENDING.load(Ordering::Acquire) > 0
}

struct Job {
    id: u64,
    method: Method,
    path: String,
    body: Option<Value>,
    token: String,
}

struct Shared {
    out: Mutex<Vec<Response>>,
    link: Mutex<Option<Link>>,
    /// the CA of the server when it is not trusted
    seen: Arc<Mutex<Option<Presented>>>,
}

pub struct Live {
    tx: Option<Sender<Job>>,
    shared: Arc<Shared>,
    url: Option<Url>,
}

/// the URL of a server address as the user typed it (bc_proto::server_address)
pub fn server_url(addr: &str) -> Option<Url> {
    Url::parse(&format!("{}/", bc_proto::server_address(addr)?))
}

impl Live {
    pub fn new(addr: &str, trust: ServerTrust) -> Live {
        let shared = Arc::new(Shared { out: Mutex::new(Vec::new()), link: Mutex::new(Some(Link::Connecting)), seen: Arc::new(Mutex::new(None)) });
        let url = server_url(addr);
        let Some(u) = url.clone() else {
            *shared.link.lock() = Some(Link::Lost);
            return Live { tx: None, shared, url: None };
        };
        let (tx, rx) = channel::<Job>(256);
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..WORKERS {
            let (rx, shared, u, t) = (rx.clone(), shared.clone(), u.clone(), trust.clone());
            let _ = plat::thread_spawn(256 * 1024, Box::new(move || worker(rx, shared, u, t)));
        }
        Live { tx: Some(tx), shared, url }
    }

    /// the server's URL (None: the address could not be read)
    pub fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }

    /// queue a request (the answer arrives through take)
    pub fn request(&mut self, id: u64, method: Method, path: &str, body: Option<Value>, token: &str) {
        let job = Job { id, method, path: String::from(path), body, token: String::from(token) };
        PENDING.fetch_add(1, Ordering::AcqRel);
        let sent = self.tx.as_ref().is_some_and(|tx| tx.send(job).is_ok());
        if !sent {
            self.shared.out.lock().push(Response { id, status: 0, body: j::obj([("message", Value::string("Ungültige Serveradresse"))]) });
        }
    }

    /// the answers that arrived
    pub fn take(&mut self) -> Vec<Response> {
        let v = core::mem::take(&mut *self.shared.out.lock());
        PENDING.fetch_sub(v.len() as u32, Ordering::AcqRel);
        v
    }

    /// a new connection state, if it changed
    pub fn link_state(&mut self) -> Option<Link> {
        self.shared.link.lock().take()
    }

    /// the CA the server presented when it was not trusted
    pub fn presented(&self) -> Option<Presented> {
        self.shared.seen.lock().clone()
    }
}

fn worker(rx: Arc<Mutex<Receiver<Job>>>, shared: Arc<Shared>, url: Url, t: ServerTrust) {
    let mut client = Client::new().with_connector(trust::connector(t, None, shared.seen.clone()));
    client.user_agent = format!("BackupCraft-Console/{}", bc_proto::VERSION);
    client.timeout_ms = 60_000;
    loop {
        let next = rx.lock().recv();
        let Some(job) = next else { return };
        let body = job.body.as_ref().map(j::text).unwrap_or_default();
        let h = netio::client::json_headers(&job.token);
        let r = client.request(job.method.as_str(), &url.with_target(&job.path), &h, body.as_bytes());
        let (resp, link) = match r {
            Ok(r) => {
                let v = if r.body_bytes().is_empty() { Value::Null } else { j::parse(r.body_bytes()).unwrap_or(Value::Null) };
                (Response { id: job.id, status: r.status, body: v }, Link::Online)
            }
            Err(HttpError::Tls(m)) if m == trust::UNTRUSTED => {
                let msg = crate::tr::t("Der Server weist sich mit einer unbekannten Zertifizierungsstelle aus");
                (Response { id: job.id, status: 0, body: j::obj([("code", Value::string("untrusted")), ("message", Value::string(msg))]) }, Link::Lost)
            }
            Err(e) => (Response { id: job.id, status: 0, body: j::obj([("message", Value::string(format!("{}", e)))]) }, Link::Lost),
        };
        shared.out.lock().push(resp);
        *shared.link.lock() = Some(link);
        plat::wake();
    }
}
