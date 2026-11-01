// webc -- the Webbrowser's side in the engine: the service "web-client" of a host (hidden: it is a
// program, no server) fetches what the browser's window asks for over the HTTP client curl uses
// (http.rs Agent), so DNS, TCP, the trace and the simulation mode see every request
//
//   Action::WebFetch { dev, req: WebReq }   a request: id (the window's own number), method, URL
//                       (http or https; a fragment is kept for the answer and not sent), headers
//                       besides Host, User-Agent and Accept where not given, body; the browser's
//                       cookies at that moment, who asks (SameSite), the certificates the user went on
//                       with.  The first one makes the service.  Refused while the device is off
//                       (ActErr).
//   Action::WebCancel { dev, id }           gives a request up: its connection closes with it, no
//                       report comes
//   Report::Web { dev, id, at, result }     the outcome (driver.rs): WebResp (the final URL, status,
//                       reason, headers, body, redirects followed, the time it took, every hop, what
//                       TLS said) or WebFail (WebErr, the URL it stopped at, why as a Msg, the hops,
//                       what TLS said)
//   Engine::web_client(dev) -> WebClientView   the requests still running, what it counted
//   Engine::service_status(dev, name)      a service's status line (Reiter Dienste, checks)
//
// Errors (WebErr): Refused (nobody listens on the port), Timeout (30 s per hop, DIAL included),
// NoName (the resolver found nothing, or no name server answered), NoRoute (no route, the network
// is down, ICMP unreachable), TooManyRedirects, Cert (the server's certificate has faults: WebFail::tls
// says which), Tls (the handshake ended otherwise: no TLS on that port, an alert), BadUrl (no http or
// https URL), Broken (the connection ended inside the answer, or the answer cannot be read),
// Stopped (the device or the service stopped under it).
//
// Cookies: the jar is the browser's window state; a request carries a snapshot of it (WebReq::jar, an
// Arc: requests of one moment share it, and K1's recording holds it, so a replay sends the same).  Every
// hop reads its Cookie header from the request's own copy at the device clock's time, and a Set-Cookie
// of a redirect changes that copy for the next hop.  The hops come back in the report with their
// Set-Cookie headers and the clock of their answers: the browser stores them in the real jar in that
// order.  The engine keeps nothing of a jar after the report.
//
// Redirects (RFC 9110 15.4): 301, 302, 303, 307 and 308 with a Location are followed, resolved
// against the URL they came from (netpkt::uri), at most MAX_REDIRECTS times: 301 and 302 turn a
// POST into a GET without its body (as browsers do), 303 turns everything but GET and HEAD into
// one, 307 and 308 keep method and body.  A Location without a fragment keeps the request's.  Other
// 3xx answers are answers.  Each redirect is a note of the device (Note::WebRedirect).
//
// At most MAX_PER_HOST connections to a host keep a page with many images polite; they stay open
// for the next page and close after IDLE without a request.  The answer
// waits inside the Agent until its last byte (footprint counts it), the Report hands it over and
// the engine keeps nothing of it.  A replay (keyframe.rs) runs the same actions; its reports go.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::cookie::{Asks, Jar, Origin};
use netpkt::http::{Headers, Version};
use netpkt::tls::{AL_BAD_CERTIFICATE, AL_CERTIFICATE_EXPIRED, AL_UNKNOWN_CA};
use netpkt::uri::Uri;

use crate::action::ActErr;
use crate::driver::Report;
pub use crate::http::TlsInfo;
use crate::http::{Agent, Done, Fail, Req, TlsReq, Url, UrlErr};
use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Ctl, Metric, OsApi, Service, SvcEvent};
use crate::tcp::TcpEnd;
use crate::trace::Note;

/// redirects one request follows at most
pub const MAX_REDIRECTS: u32 = 10;
/// the name of the service on a device
pub const CLIENT_SVC: &str = "webbrowser";
const TIMEOUT: Ns = 30 * SEC;
const IDLE: Ns = 30 * SEC;
const MAX_PER_HOST: usize = 6;
const K_HTTP: u32 = 0;
const USER_AGENT: &str = "JFN-Browser/1.0";

/// a request of the browser (Action::WebFetch)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebReq {
    /// the window's number for it: the report carries it back
    pub id: u64,
    pub method: String,
    pub url: String,
    pub headers: Headers,
    pub body: Vec<u8>,
    /// the browser's cookies when it asked (None: none go with it)
    pub jar: Option<Arc<Jar>>,
    /// the site of the page that asks ("" : the user, by the address line)
    pub site: String,
    /// a navigation of the window (a link, a form); false: a picture, a sheet, a script's request
    pub top: bool,
    /// certificates the user went on with: SHA-256 of the leaf
    pub accept: Vec<[u8; 32]>,
}

/// a hop of a request: what went and what came
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebHop {
    pub method: String,
    pub url: String,
    /// the head as it went, Host first
    pub sent: Headers,
    /// 0: no answer came
    pub status: u16,
    pub reason: String,
    pub headers: Headers,
    /// the device clock when its answer came, in seconds since 1970 (what Max-Age counts from)
    pub clock: i64,
    /// from its start to its end
    pub took: Ns,
    pub tls: Option<TlsInfo>,
}

/// why a request brought no answer
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WebErr {
    Refused,
    Timeout,
    NoName,
    NoRoute,
    TooManyRedirects,
    /// the server's certificate has faults (WebFail::tls)
    Cert,
    /// TLS ended otherwise
    Tls,
    BadUrl,
    Broken,
    Stopped,
}

/// a request that brought no answer
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebFail {
    pub kind: WebErr,
    /// the URL it stopped at (after the redirects)
    pub url: String,
    pub msg: Msg,
    /// its hops, the one that failed last (status 0)
    pub hops: Vec<WebHop>,
    /// what TLS said on the last hop
    pub tls: Option<TlsInfo>,
}

/// an answer
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebResp {
    /// the final URL (after the redirects, with the fragment)
    pub url: String,
    pub status: u16,
    pub reason: String,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub redirects: u32,
    /// from the request to the last byte of the answer
    pub took: Ns,
    /// its hops, the final one last
    pub hops: Vec<WebHop>,
    /// what TLS said on the last hop (None: no TLS)
    pub tls: Option<TlsInfo>,
}

/// the service as the window asks about it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebClientView {
    /// the ids of the requests still running or waiting
    pub active: Vec<u64>,
    pub requests: u64,
    pub responses: u64,
    pub failed: u64,
    pub redirects: u64,
}

/// a request on its way
#[derive(Clone, Debug)]
struct Job {
    id: u64,
    method: String,
    /// where it goes now, without a fragment
    url: String,
    frag: Option<String>,
    headers: Headers,
    body: Vec<u8>,
    hops: u32,
    start: Ns,
    /// a copy of the browser's cookies: Set-Cookie on the way changes it
    jar: Option<Arc<Jar>>,
    site: String,
    top: bool,
    accept: Vec<[u8; 32]>,
    /// the hops so far, and the head of the one on its way
    done: Vec<WebHop>,
    sent: Headers,
    hop_start: Ns,
    secure: bool,
}

/// the service of the Webbrowser (see the module header)
#[derive(Clone, Debug)]
pub struct WebClient {
    agent: Agent,
    /// actions that came since the service last ran
    queue: VecDeque<WebReq>,
    cancels: Vec<u64>,
    /// the requests in the Agent, by its id
    jobs: BTreeMap<u64, Job>,
    pub requests: u64,
    pub responses: u64,
    pub failed: u64,
    pub redirects: u64,
}

impl Default for WebClient {
    fn default() -> WebClient {
        WebClient::new()
    }
}

impl WebClient {
    pub fn new() -> WebClient {
        let mut agent = Agent::new(K_HTTP);
        agent.max_per_host = MAX_PER_HOST;
        agent.idle_timeout = IDLE;
        WebClient { agent, queue: VecDeque::new(), cancels: Vec::new(), jobs: BTreeMap::new(), requests: 0, responses: 0, failed: 0, redirects: 0 }
    }

    /// the view for the window
    pub fn view(&self) -> WebClientView {
        let mut active: Vec<u64> = self.queue.iter().map(|q| q.id).chain(self.jobs.values().map(|j| j.id)).collect();
        active.sort_unstable();
        WebClientView { active, requests: self.requests, responses: self.responses, failed: self.failed, redirects: self.redirects }
    }

    fn report(&mut self, os: &mut OsApi, id: u64, result: Result<WebResp, WebFail>) {
        match &result {
            Ok(_) => self.responses += 1,
            Err(_) => self.failed += 1,
        }
        let (dev, at) = (os.dev(), os.now());
        os.eng.cli.reports.push(Report::Web { dev, id, at, result });
    }

    /// what the window asked for since the service last ran
    fn kick(&mut self, os: &mut OsApi) {
        for id in core::mem::take(&mut self.cancels) {
            self.queue.retain(|q| q.id != id);
            let aid = self.jobs.iter().find(|(_, j)| j.id == id).map(|(a, _)| *a);
            if let Some(a) = aid {
                self.jobs.remove(&a);
                self.agent.cancel(os, a);
            }
        }
        while let Some(q) = self.queue.pop_front() {
            self.requests += 1;
            let u = Uri::parse(q.url.trim()).encoded();
            let frag = u.fragment.clone();
            let now = os.now();
            let job = Job { id: q.id, method: q.method, url: u.text(), frag, headers: q.headers, body: q.body, hops: 0, start: now, jar: q.jar, site: q.site, top: q.top, accept: q.accept, done: Vec::new(), sent: Headers::new(), hop_start: now, secure: false };
            self.send(os, job);
        }
        self.pump(os);
    }

    /// a request goes to the Agent, when its URL is one for http
    fn send(&mut self, os: &mut OsApi, mut job: Job) {
        let (url, secure) = match web_url(&job.url) {
            Ok(u) => u,
            Err(e) => return self.fail(os, job, e, None),
        };
        let mut h = Headers::new();
        h.add("User-Agent", USER_AGENT);
        if job.headers.get("accept").is_none() {
            h.add("Accept", "*/*");
        }
        for (k, v) in &job.headers.0 {
            h.set(k, v);
        }
        if let Some(jar) = &job.jar {
            let clock = os.clock().div_euclid(1_000_000_000);
            let safe = job.method.eq_ignore_ascii_case("GET") || job.method.eq_ignore_ascii_case("HEAD");
            let site = (!job.site.is_empty()).then_some(job.site.as_str());
            let asks = Asks { host: &url.host, path: path_of(&url.target), secure, http: true, site, top: job.top, safe };
            let c = jar.header(&asks, clock);
            if !c.is_empty() {
                h.set("Cookie", &c);
            }
        }
        // the head as it goes, for the window's network log (the Agent puts Host first)
        let mut sent = Headers::new();
        sent.add("Host", &if secure { url.host_header_tls() } else { url.host_header() });
        for (k, v) in &h.0 {
            sent.add(k, v);
        }
        if !job.body.is_empty() || matches!(job.method.as_str(), "POST" | "PUT" | "PATCH") {
            sent.add("Content-Length", &format!("{}", job.body.len()));
        }
        job.sent = sent;
        job.secure = secure;
        job.hop_start = os.now();
        let tls = secure.then(|| TlsReq { insecure: false, accept: job.accept.clone() });
        let req = Req { method: job.method.clone(), url, version: Version::Http11, headers: h, body: job.body.clone(), timeout: TIMEOUT, connect_timeout: 0, reuse: true, keep_body: true };
        let aid = self.agent.submit_tls(os, req, tls);
        self.jobs.insert(aid, job);
    }

    fn fail(&mut self, os: &mut OsApi, mut job: Job, e: (WebErr, Msg), tls: Option<TlsInfo>) {
        let id = job.id;
        let now = os.now();
        let hop = WebHop { method: job.method.clone(), url: job.url.clone(), sent: core::mem::take(&mut job.sent), clock: os.clock().div_euclid(1_000_000_000), took: now.saturating_sub(job.hop_start), tls: tls.clone(), ..WebHop::default() };
        job.done.push(hop);
        self.report(os, id, Err(WebFail { kind: e.0, url: job.url, msg: e.1, hops: job.done, tls }));
    }

    /// the requests that finished (a failure can come at once from the Agent, so this runs until
    /// none is left)
    fn pump(&mut self, os: &mut OsApi) {
        loop {
            let done = self.agent.take_done();
            if done.is_empty() {
                return;
            }
            for d in done {
                self.finished(os, d);
            }
        }
    }

    fn finished(&mut self, os: &mut OsApi, d: Done) {
        let Some(mut job) = self.jobs.remove(&d.id) else { return };
        let r = match d.result {
            Ok(r) => r,
            Err(f) => {
                let kind = kind_of(&f);
                return self.fail(os, job, (kind, f.msg()), d.tls);
            }
        };
        let status = r.head.status;
        let clock = os.clock().div_euclid(1_000_000_000);
        // what this hop sets counts for the next one
        if let Some(jar) = job.jar.as_mut() {
            let setting: Vec<&str> = r.head.headers.0.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.as_str()).collect();
            if !setting.is_empty() {
                let j = Arc::make_mut(jar);
                let o = Origin { host: &d.url.host, path: path_of(&d.url.target), secure: job.secure, http: true };
                for v in setting {
                    let _ = j.store(v, o, clock);
                }
            }
        }
        let hop = WebHop { method: job.method.clone(), url: job.url.clone(), sent: core::mem::take(&mut job.sent), status, reason: r.head.reason.clone(), headers: r.head.headers.clone(), clock, took: d.end.saturating_sub(job.hop_start), tls: d.tls.clone() };
        job.done.push(hop);
        let loc = if matches!(status, 301 | 302 | 303 | 307 | 308) { r.head.headers.get("location") } else { None };
        if let Some(l) = loc {
            if job.hops >= MAX_REDIRECTS {
                return self.fail(os, job, (WebErr::TooManyRedirects, msg!("Nach {} Weiterleitungen kam noch eine: es bleibt dabei.", MAX_REDIRECTS)), d.tls);
            }
            let t = Uri::parse(&job.url).join(l.trim()).encoded();
            if t.fragment.is_some() {
                job.frag = t.fragment.clone();
            }
            let to = Uri { fragment: None, ..t }.text();
            (job.method, job.body) = redirected(status, job.method, job.body);
            if job.body.is_empty() {
                job.headers.remove("content-type");
            }
            job.url = to.clone();
            job.hops += 1;
            self.redirects += 1;
            os.note(Note::WebRedirect { status, to });
            return self.send(os, job);
        }
        let base = d.url.text_web(job.secure);
        let url = match &job.frag {
            Some(f) => alloc::format!("{}#{}", base, f),
            None => base,
        };
        let took = d.end.saturating_sub(job.start);
        let id = job.id;
        self.report(os, id, Ok(WebResp { url, status, reason: r.head.reason, headers: r.head.headers, body: r.body, redirects: job.hops, took, hops: job.done, tls: d.tls }));
    }
}

/// what a redirect makes of method and body (see the module header)
pub fn redirected(status: u16, method: String, body: Vec<u8>) -> (String, Vec<u8>) {
    let get = match status {
        301 | 302 => method.eq_ignore_ascii_case("POST"),
        303 => !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("HEAD"),
        _ => false,
    };
    if get { (String::from("GET"), Vec::new()) } else { (method, body) }
}

/// the error kind of a failure of the HTTP client
pub fn kind_of(f: &Fail) -> WebErr {
    match f {
        Fail::Resolve(_) => WebErr::NoName,
        Fail::NoRoute(_) => WebErr::NoRoute,
        Fail::ConnectTimeout | Fail::Timeout { .. } => WebErr::Timeout,
        Fail::Connect(TcpEnd::Refused) => WebErr::Refused,
        Fail::Connect(TcpEnd::Timeout) | Fail::Reset(TcpEnd::Timeout) => WebErr::Timeout,
        Fail::Connect(TcpEnd::Unreachable) | Fail::Reset(TcpEnd::Unreachable) => WebErr::NoRoute,
        Fail::Tls { alert: AL_BAD_CERTIFICATE | AL_CERTIFICATE_EXPIRED | AL_UNKNOWN_CA, ours: true, .. } => WebErr::Cert,
        Fail::Tls { .. } => WebErr::Tls,
        _ => WebErr::Broken,
    }
}

// the path of a target, without its query
fn path_of(target: &str) -> &str {
    target.split('?').next().unwrap_or("/")
}

// the URL for the Agent and whether it is https, or why it is none
fn web_url(s: &str) -> Result<(Url, bool), (WebErr, Msg)> {
    Url::parse_web(s).map_err(|e| match e {
        UrlErr::Scheme(x) => (WebErr::BadUrl, msg!("\u{201e}{}\u{201c}: nur http und https gehen.", x)),
        UrlErr::NoHost => (WebErr::BadUrl, msg!("Die URL nennt keinen Host.")),
        UrlErr::Port => (WebErr::BadUrl, msg!("Der Port der URL ist keine Zahl bis 65535.")),
        UrlErr::BadHost => (WebErr::BadUrl, msg!("Der Host der URL hat ungültige Zeichen.")),
    })
}

impl Service for WebClient {
    fn kind(&self) -> &'static str {
        "web-client"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.kick(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if let SvcEvent::Ctl(Ctl::Renew) = ev {
            self.kick(os);
            return;
        }
        if self.agent.on(&ev, os) {
            self.pump(os);
        }
    }
    fn stop(&mut self, os: &mut OsApi) {
        // the window hears of what ran and what waited
        let m = msg!("Das Gerät oder der Dienst hat angehalten.");
        for job in core::mem::take(&mut self.jobs).into_values() {
            self.fail(os, job, (WebErr::Stopped, m.clone()), None);
        }
        for q in core::mem::take(&mut self.queue) {
            self.requests += 1;
            self.report(os, q.id, Err(WebFail { kind: WebErr::Stopped, url: q.url, msg: m.clone(), hops: Vec::new(), tls: None }));
        }
        self.agent.reset();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("requests", self.requests as i64),
            Metric::new("responses", self.responses as i64),
            Metric::new("failed", self.failed as i64),
            Metric::new("redirects", self.redirects as i64),
            Metric::new("active", (self.jobs.len() + self.queue.len()) as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("{} Anfragen, {} Antworten, {} gescheitert", self.requests, self.responses, self.failed)
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn footprint(&self) -> usize {
        let jar = |j: &Option<Arc<Jar>>| j.as_ref().map_or(0, |x| x.bytes());
        let hop = |h: &WebHop| h.url.len() + h.sent.0.len() * 64 + h.headers.0.len() * 64 + h.tls.as_ref().map_or(0, |t| t.chain.iter().map(|c| c.bytes()).sum::<usize>()) + 128;
        let job = |j: &Job| j.url.len() + j.body.len() + 128 + jar(&j.jar) + j.done.iter().map(hop).sum::<usize>();
        512 + self.agent.held_bytes() + self.jobs.values().map(job).sum::<usize>() + self.queue.iter().map(|q| q.url.len() + q.body.len() + 128 + jar(&q.jar)).sum::<usize>()
    }
}

impl Engine {
    // the running device's service of the browser, made on the first request, and whether it runs
    fn web_slot(&mut self, dev: u32) -> Result<(usize, u32), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        if !matches!(self.nodes.get(ni), Some(Some(n)) if n.power) {
            return Err(ActErr::Refused(msg!("Das Gerät ist ausgeschaltet.")));
        }
        let have = self.os(ni).and_then(|o| o.svc_id(CLIENT_SVC));
        let id = match have {
            Some(id) => id,
            None => {
                self.add_service(dev, CLIENT_SVC, Box::new(WebClient::new()), true)?;
                let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
                let s = os.svcs.iter_mut().find(|s| s.name == CLIENT_SVC).ok_or(ActErr::NoIp(dev))?;
                // a program, not a server: not among the device's services
                s.console = true;
                s.id
            }
        };
        Ok((ni, id))
    }

    // the service tells of what the window asked for (or starts when it does not run yet)
    fn web_wake(&mut self, ni: usize, id: u32) {
        let running = self.os(ni).is_some_and(|o| o.running(id));
        if running {
            self.svc_ctl(ni, id, Ctl::Renew);
        }
    }

    fn web_of(&mut self, ni: usize, id: u32) -> Result<&mut WebClient, ActErr> {
        let refused = || ActErr::Refused(msg!("Der Dienst \u{201e}{}\u{201c} ist von anderer Art.", CLIENT_SVC));
        let os = self.os_mut(ni).ok_or_else(refused)?;
        let slot = os.svcs.iter_mut().find(|s| s.id == id).ok_or_else(refused)?;
        slot.svc.as_mut().and_then(|x| x.as_any_mut().downcast_mut::<WebClient>()).ok_or_else(refused)
    }

    /// Action::WebFetch
    pub(crate) fn web_fetch(&mut self, dev: u32, req: WebReq) -> Result<(), ActErr> {
        let (ni, id) = self.web_slot(dev)?;
        self.web_of(ni, id)?.queue.push_back(req);
        self.web_wake(ni, id);
        Ok(())
    }

    /// Action::WebCancel
    pub(crate) fn web_cancel(&mut self, dev: u32, req: u64) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(CLIENT_SVC)) else { return Ok(()) };
        self.web_of(ni, id)?.cancels.push(req);
        self.web_wake(ni, id);
        Ok(())
    }

    /// a service's status line (None: no such service)
    pub fn service_status(&self, dev: u32, name: &str) -> Option<String> {
        let os = self.os(self.node_index(dev)?)?;
        Some(os.svc(name)?.svc.as_ref()?.status())
    }

    /// the Webbrowser's service of a device (None: it never ran)
    pub fn web_client(&self, dev: u32) -> Option<WebClientView> {
        let os = self.os(self.node_index(dev)?)?;
        os.svc(CLIENT_SVC)?.svc.as_ref()?.as_any().downcast_ref::<WebClient>().map(WebClient::view)
    }
}
