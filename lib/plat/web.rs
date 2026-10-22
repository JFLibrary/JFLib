// Shared browser/Android host adapter. The host exchanges bounded buffers and queued events;
// app state stays in the same Rust UI/runtime used by native applications.
// Android differs where its system offers more: real files on internal storage
// (android_files.rs), the documents of other apps as content:// names, the system clipboard and
// the persisted grants (android_host.rs), the system's file picker behind file_dialog
// (FD_PENDING, file_dialog_poll), download() as "save a copy" through the picker.

use crate::{Capabilities, Date, Event};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};
use rt::sync::SpinLock;

// (two declarations, not cfg_attr: tools/gendoc.py follows #[path] only)
#[cfg(target_os = "android")]
#[path = "android_files.rs"]
mod files;
#[cfg(not(target_os = "android"))]
#[path = "web_files.rs"]
mod files;
pub use files::*;
#[cfg(target_os = "android")]
#[path = "android_host.rs"]
pub(crate) mod android_host;
#[cfg(target_os = "android")]
pub use android_host::{content_fs, content_grants, content_release, jfgui_reply, AndroidDocs};
#[path = "web_windows.rs"]
mod windows;
pub use windows::*;
#[path = "web_a11y.rs"]
pub(crate) mod a11y_host;
#[cfg(target_os = "android")]
#[path = "android_ime.rs"]
mod android_ime;
#[cfg(target_os = "android")]
pub use android_ime::*;
#[cfg(web_threads)]
#[path = "web_threads.rs"]
mod threads;
#[cfg(web_threads)]
pub use threads::{Sem, Thread, spawn as thread_spawn};

#[cfg_attr(target_arch = "wasm32", link(wasm_import_module = "jfgui"))]
unsafe extern "C" {
    #[cfg_attr(target_arch = "wasm32", link_name = "time_us")]
    #[cfg_attr(target_os = "android", link_name = "jfgui_host_time_us")]
    fn host_time_us() -> f64;
    #[cfg_attr(target_arch = "wasm32", link_name = "present")]
    #[cfg_attr(target_os = "android", link_name = "jfgui_host_present")]
    fn host_present(ptr: *const u8, len: usize, width: i32, height: i32, stride: i32, rects: *const [i32; 4], count: usize);
    #[cfg_attr(target_arch = "wasm32", link_name = "request")]
    #[cfg_attr(target_os = "android", link_name = "jfgui_host_request")]
    fn host_request(kind: u32, id: u32, ptr: *const u8, len: usize);
}
unsafe extern "Rust" {
    fn jfgui_main() -> i32;
}

/// Persist a virtual file to browser storage; payload: u32 path byte count, path, file bytes.
pub const REQUEST_PERSIST: u32 = 1;
/// Export/download a file; same payload framing as persistence.
pub const REQUEST_DOWNLOAD: u32 = 2;
/// Open a browser file picker; payload is a UTF-8 extension filter.
pub const REQUEST_OPEN: u32 = 3;
/// Read browser clipboard text asynchronously.
pub const REQUEST_CLIP_READ: u32 = 4;
/// Write browser clipboard text; payload is UTF-8.
pub const REQUEST_CLIP_WRITE: u32 = 5;
/// Update the DOM composition element's caret rectangle (three little-endian i32 values).
pub const REQUEST_IME: u32 = 6;
/// Update the browser pointer shape (one little-endian u32 CUR value).
pub const REQUEST_CURSOR: u32 = 7;
/// Update the document title (UTF-8).
pub const REQUEST_TITLE: u32 = 8;
/// Open an explicitly requested HTTP(S)/mailto URL; payload is UTF-8.
pub const REQUEST_URL: u32 = 9;
/// Mirror explicit in-app control activation/revocation to the host (one boolean byte).
pub const REQUEST_AGENT: u32 = 10;
/// The differences of the accessibility tree for the host's mirror (web_a11y.rs, a11y_wire.rs).
pub const REQUEST_A11Y: u32 = 11;
/// (Android) the system's file picker for file_dialog: u32 flags, the suggested name, the
/// content:// URI it starts at (may be empty), u32 count, the extensions (one string each,
/// "png"); answered by jfgui_file_result with the chosen content:// URI
pub const REQUEST_FILE_DIALOG: u32 = 12;
/// (browser) commit a change of the secret store (secret.rs, web_secret.rs): u32 op (1 set,
/// 2 delete), app, service, account and for set the value, each as u32 length + bytes; the
/// host encrypts and stores it in order and completes with 2, or an error
pub const REQUEST_SECRET: u32 = 13;

/// Completion state of one browser operation; durable and exported are distinct outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestStatus {
    /// The host has not completed the operation.
    Pending,
    /// Browser storage confirmed this exact file revision was committed.
    Durable,
    /// The host completed a download/export, not an origin-storage commit.
    Exported,
    /// A transient operation such as clipboard access completed.
    Completed,
    /// The user or browser denied access.
    Denied,
    /// Browser storage quota was exhausted.
    Quota,
    /// This browser lacks the requested service.
    Unsupported,
    /// User cancellation.
    Cancelled,
    /// Other host I/O failure.
    Failed,
}
struct Request {
    id: u32,
    kind: u32,
    status: RequestStatus,
    file: Option<(String, u64)>,
    detached: bool,
    // (Android) the picker's answer: the chosen content:// URI
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    result: Option<String>,
}
static REQUESTS: SpinLock<Vec<Request>> = SpinLock::new(Vec::new());
static NEXT_REQUEST: AtomicU32 = AtomicU32::new(1);
static CAP_EXTRA: AtomicU32 = AtomicU32::new(0);
static ARGS: SpinLock<Vec<String>> = SpinLock::new(Vec::new());
static ENV: SpinLock<Vec<(String, String)>> = SpinLock::new(Vec::new());
static INBOX: SpinLock<Vec<u8>> = SpinLock::new(Vec::new());
static OUTBOX: SpinLock<Vec<u8>> = SpinLock::new(Vec::new());
static CLIP: SpinLock<Vec<(u32, Vec<u8>)>> = SpinLock::new(Vec::new());
static CALENDAR: SpinLock<Date> = SpinLock::new(Date {
    year: 1970,
    month: 1,
    day: 1,
    hour: 0,
    min: 0,
    sec: 0,
    ms: 0,
    wday: 4,
});
static STARTED: AtomicBool = AtomicBool::new(false);
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);
static AGENT: AtomicBool = AtomicBool::new(false);

/// An owned app runtime registered once when the browser entrypoint returns.
pub trait WebApplication {
    /// Milliseconds until a timer or animation needs a host tick; input wakes immediately.
    fn frame_delay(&self) -> u32 { 16 }
    /// Drain queued input without presenting intermediate frames; default runtimes may tick.
    fn input(&mut self, time_ms: u32) { self.tick(time_ms); }
    /// Process queued input, advance timers and render one frame.
    fn tick(&mut self, time_ms: u32);
    /// Process one explicitly enabled agent request, returning its UTF-8 response.
    fn control(&mut self, line: &[u8]) -> Option<String>;
    /// Synchronize runtime visibility and command state when the host changes activation.
    fn set_control_enabled(&mut self, _enabled: bool) {}
}
struct AppSlot(UnsafeCell<Option<Box<dyn WebApplication>>>);
// SAFETY: a WASM instance belongs to one worker, an Android instance to the Java UI thread;
// exports take the app out before running
// callbacks, so a synchronous host re-entry observes None instead of aliasing the app.
unsafe impl Sync for AppSlot {}
static APP: AppSlot = AppSlot(UnsafeCell::new(None));

/// Install the owned runtime. Returns false if an app is already registered.
pub fn install(app: Box<dyn WebApplication>) -> bool {
    // SAFETY: the instance is single-threaded and no borrow survives this function.
    let slot = unsafe { &mut *APP.0.get() };
    if slot.is_some() {
        return false;
    }
    *slot = Some(app);
    true
}
fn with_app<R>(f: impl FnOnce(&mut dyn WebApplication) -> R) -> Option<R> {
    // SAFETY: take excludes nested calls; callbacks run with no APP reference alive.
    let mut app = unsafe { (&mut *APP.0.get()).take() }?;
    let result = f(&mut *app);
    // SAFETY: all app mutation finished and the slot is still empty.
    unsafe {
        *APP.0.get() = Some(app);
    }
    Some(result)
}

pub(super) fn notice(kind: u32, data: &[u8]) {
    // SAFETY: host copies bytes synchronously and schedules any completion asynchronously.
    unsafe {
        host_request(kind, 0, data.as_ptr(), data.len());
    }
}
pub(super) fn present_bytes(data: &[u8], width: i32, height: i32, stride: i32, rects: &[[i32; 4]]) {
    // SAFETY: the host copies both valid slices before returning; no retained linear-memory views.
    unsafe {
        host_present(data.as_ptr(), data.len(), width, height, stride, rects.as_ptr(), rects.len());
    }
}
pub(crate) fn request(kind: u32, bytes: &[u8], file: Option<(String, u64)>) -> Option<u32> {
    let id = {
        let mut pending = REQUESTS.lock();
        if pending.len() >= 256 {
            // Keep a bounded terminal-task history so a long-lived app never exhausts
            // its request slots just because agents did not explicitly forget results.
            let oldest = pending
                .iter()
                .position(|r| r.status != RequestStatus::Pending)?;
            pending.remove(oldest);
        }
        let id = NEXT_REQUEST
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .ok()?;
        pending.push(Request {
            id,
            kind,
            status: RequestStatus::Pending,
            file,
            detached: matches!(kind, REQUEST_URL | REQUEST_CLIP_WRITE | REQUEST_SECRET),
            result: None,
        });
        id
    };
    // SAFETY: borrowed bytes are copied before this synchronous import returns.
    unsafe {
        host_request(kind, id, bytes.as_ptr(), bytes.len());
    }
    Some(id)
}
fn file_request(kind: u32, path: &str) -> Option<u32> {
    let (path, revision, data) = files::snapshot(path)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(4 + path.len() + data.len()).ok()?;
    bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
    bytes.extend_from_slice(path.as_bytes());
    bytes.extend_from_slice(&data);
    request(kind, &bytes, Some((path, revision)))
}
/// Request durable origin storage for the current virtual-file revision.
#[cfg(not(target_os = "android"))]
pub fn persist(path: &str) -> Option<u32> {
    file_request(REQUEST_PERSIST, path)
}
/// Android: the file is on internal storage already; its data is synced (fsync) and the
/// request is Durable (Failed for a missing file) at once.
#[cfg(target_os = "android")]
pub fn persist(path: &str) -> Option<u32> {
    let ok = files::sync_path(path);
    let id = NEXT_REQUEST.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1)).ok()?;
    let mut pending = REQUESTS.lock();
    if pending.len() >= 256 {
        let oldest = pending.iter().position(|r| r.status != RequestStatus::Pending)?;
        pending.remove(oldest);
    }
    let status = if ok { RequestStatus::Durable } else { RequestStatus::Failed };
    pending.push(Request { id, kind: REQUEST_PERSIST, status, file: None, detached: false, result: None });
    Some(id)
}
/// Request a browser download; success means export, not durable origin storage.  Android:
/// "save a copy" through the system's picker (the user chooses where).
pub fn download(path: &str) -> Option<u32> {
    file_request(REQUEST_DOWNLOAD, path)
}
/// Request file selection. The host imports selected files before completing the request.
/// Android: the chosen document arrives as its content:// name in a drop (E_DROP).
pub fn pick_file(extensions: &str) -> Option<u32> {
    request(REQUEST_OPEN, extensions.as_bytes(), None)
}
/// Query an operation. Old terminal results expire when the 256-entry history fills.
pub fn request_status(id: u32) -> Option<RequestStatus> {
    REQUESTS
        .lock()
        .iter()
        .find(|r| r.id == id)
        .map(|r| r.status)
}
/// Remove a completed operation once its result has been consumed.
pub fn forget_request(id: u32) -> bool {
    let mut requests = REQUESTS.lock();
    if let Some(i) = requests
        .iter()
        .position(|r| r.id == id && r.status != RequestStatus::Pending)
    {
        requests.remove(i);
        true
    } else {
        false
    }
}

/// Services supplied by this browser session, including dynamically granted host services.
pub fn capabilities() -> Capabilities {
    Capabilities {
        available: (crate::CAP_FILE_READ
            | crate::CAP_FILE_WRITE
            // Rich formats are instance-local. System text writes need browser permission.
            | crate::CAP_CLIPBOARD_RICH
            | crate::CAP_WINDOWING
            | if cfg!(web_threads) { crate::CAP_THREADS } else { 0 }
            // Android: internal storage and the ClipboardManager (images: GalleryFiles)
            | if cfg!(target_os = "android") {
                crate::CAP_PERSISTENT_STORAGE | crate::CAP_CLIPBOARD_TEXT | crate::CAP_CLIPBOARD_IMAGE
            } else {
                0
            }
            | CAP_EXTRA.load(Ordering::Relaxed) as u64)
            & crate::policy::cap_mask(),
    }
}
/// App start arguments copied from the host, with a synthetic executable name first.
pub fn args() -> Vec<String> {
    ARGS.lock().clone()
}
/// Monotonic microseconds from the host's performance clock.
pub fn time_us() -> u64 {
    // SAFETY: scalar return without memory access.
    unsafe { host_time_us().max(0.0) as u64 }
}
/// No blocking sleep occurs in the browser; callers must return to their step loop.
pub fn sleep_ms(_ms: u32) {}
/// Read a per-instance virtual environment setting.
pub fn env(name: &str) -> Option<String> {
    ENV.lock()
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, v)| v.clone())
}
/// Alias for the shared environment API.
pub fn getenv(name: &str) -> Option<String> {
    env(name)
}
/// Set a per-instance environment value without changing the browser or OS environment.
pub fn setenv(name: &str, value: Option<&str>) -> bool {
    if name.is_empty() || name.contains('\0') || value.is_some_and(|v| v.contains('\0')) {
        return false;
    }
    let mut values = ENV.lock();
    values.retain(|(key, _)| key != name);
    if let Some(value) = value {
        values.push((String::from(name), String::from(value)));
    }
    true
}
/// Host-supplied UTC calendar, updated independently from the monotonic clock.
pub fn date() -> (Date, bool) {
    (*CALENDAR.lock(), true)
}
/// Wall-clock time from the host calendar: nanoseconds since 1970-01-01 00:00 UTC.
pub fn unix_time_ns() -> i64 {
    let d = *CALENDAR.lock();
    let (y, m) = (d.year as i64, d.month as i64);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d.day as i64 - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    let secs = days * 86_400 + d.hour as i64 * 3600 + d.min as i64 * 60 + d.sec as i64;
    secs * 1_000_000_000 + d.ms as i64 * 1_000_000
}
/// Theme requested by the host: 0 light, 1 dark, or -1 unspecified.
pub fn theme() -> i32 {
    match env("jfgui_THEME").as_deref() {
        Some("dark") => 1,
        Some("light") => 0,
        _ => -1,
    }
}
/// Available compute workers in a shared-memory bundle, otherwise one app worker.
pub fn cpu_count() -> u32 {
    #[cfg(web_threads)]
    { threads::cpu_count() }
    #[cfg(not(web_threads))]
    { 1 }
}
/// Current WASM linear-memory allocation; zero when the embedded host does not report RAM.
pub fn mem_total() -> u64 {
    #[cfg(target_arch = "wasm32")]
    { core::arch::wasm32::memory_size::<0>() as u64 * 65536 }
    #[cfg(target_os = "android")]
    { 0 }
}
/// Cooperative yield is achieved by returning from the current runtime step.
pub fn thread_yield() {}
/// Browser thread handle placeholder; spawning native threads is unsupported.
#[cfg(not(web_threads))]
pub struct Thread;
#[cfg(not(web_threads))]
impl Thread {
    /// No thread was started.
    pub fn join(self) {}
}
/// Reject native thread creation. Use Job for cooperative background work.
#[cfg(not(web_threads))]
pub fn thread_spawn(_stack: usize, _f: Box<dyn FnOnce() + Send>) -> Option<Thread> {
    None
}
/// Nonblocking counting semaphore used only by portable try-wait paths.
#[cfg(not(web_threads))]
pub struct Sem(AtomicU32);
#[cfg(not(web_threads))]
impl Sem {
    /// Create a logical semaphore inside this worker.
    pub fn new(initial: u32) -> Option<Self> {
        Some(Self(AtomicU32::new(initial)))
    }
    /// Add available units.
    pub fn post(&self, count: u32) {
        self.0.fetch_add(count, Ordering::Relaxed);
    }
    /// Try to take a unit; browser execution never blocks on another thread.
    pub fn wait(&self, _timeout: i32) -> bool {
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(1))
            .is_ok()
    }
}
/// Coalesce job/repaint notifications without occupying the bounded input ring.
pub fn wake() {
    WAKE_PENDING.store(true, Ordering::Release);
}
fn drain_wake() {
    if WAKE_PENDING.swap(false, Ordering::AcqRel)
        && !crate::evq_push(Event { typ: crate::E_WAKE, ..Event::default() })
    {
        WAKE_PENDING.store(true, Ordering::Release);
    }
}
/// Poll available events; the host schedules future ticks.
pub fn wait(_deadline_us: u64) -> u32 {
    crate::evq_count()
}
/// Dispatch already queued events without blocking the browser worker.
pub fn wait_events(_deadline_us: u64, on_event: &mut dyn FnMut(&Event)) -> u32 {
    let mut count = 0;
    while let Some(event) = crate::evq_pop() {
        on_event(&event);
        count += 1;
    }
    count
}
/// Browser MessagePort replaces process stdin.
pub fn read_stdin(_bytes: &mut [u8]) -> Result<usize, u32> {
    Err(38)
}
/// The browser uses the shared virtual file dialog; its native synchronous picker is unavailable.
#[cfg(not(target_os = "android"))]
pub fn file_dialog(_flags: u32, _path: &mut String, _extensions: &[&str]) -> (i32, u32) {
    (-1, 0)
}
/// Android: the system's picker (Storage Access Framework) -- FD_OPEN ACTION_OPEN_DOCUMENT,
/// FD_SAVE ACTION_CREATE_DOCUMENT (the suggested name from path, the type of the selected
/// extension), FD_FOLDER ACTION_OPEN_DOCUMENT_TREE.  It answers later: (FD_PENDING, 0) now,
/// file_dialog_poll() once the user is done (the UI wakes up then); the chosen document comes
/// as its content:// name, its grant kept across restarts.  One picker at a time: a second
/// call while one is open answers (0, 0).  (-1, 0) when the host has no picker.
#[cfg(target_os = "android")]
pub fn file_dialog(flags: u32, path: &mut String, extensions: &[&str]) -> (i32, u32) {
    let mut slot = DIALOG.lock();
    if let Some(id) = *slot {
        if request_status(id) == Some(RequestStatus::Pending) {
            return (0, 0);
        }
        forget_request(id);
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&flags.to_le_bytes());
    // the suggested name and where the picker starts: a content:// name -> its document (save
    // as: a new name in a chosen folder -> that folder), else the name's last segment
    let (mut name, mut start) = (String::from(path.rsplit(['/', '\\']).next().unwrap_or("")), String::new());
    if let Some(rest) = path.strip_prefix("content://") {
        let docs = crate::content::ContentFs::new(AndroidDocs);
        if let Some(d) = docs.document(rest) {
            name = if d.dir { String::new() } else { d.name };
            start = d.uri;
        } else if let Some(d) = rest.rsplit_once('/').and_then(|(dir, _)| docs.document(dir)).filter(|d| d.dir) {
            start = d.uri;
        } else {
            name.clear();
            start = path.clone(); // an Android URI the app holds no grant for yet: the picker may know it
        }
    }
    for s in [&name, &start] {
        bytes.extend_from_slice(&(s.len() as u32).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    // every extension of every entry ("a;b=description"), the selected entry's first
    let exts: Vec<&str> = extensions.iter().take_while(|e| !e.is_empty()).copied().collect();
    let sel = ((flags >> crate::FD_EXT_SHIFT) & 0xFF) as usize;
    let order = exts.get(sel).into_iter().chain(exts.iter().enumerate().filter(|(k, _)| *k != sel).map(|(_, e)| e));
    let mut list: Vec<&str> = Vec::new();
    for e in order {
        for x in e.split('=').next().unwrap_or("").split(';').filter(|x| !x.is_empty()) {
            if !list.contains(&x) {
                list.push(x);
            }
        }
    }
    bytes.extend_from_slice(&(list.len() as u32).to_le_bytes());
    for x in &list {
        bytes.extend_from_slice(&(x.len() as u32).to_le_bytes());
        bytes.extend_from_slice(x.as_bytes());
    }
    let Some(id) = request(REQUEST_FILE_DIALOG, &bytes, None) else { return (-1, 0) };
    if request_status(id) == Some(RequestStatus::Unsupported) {
        forget_request(id);
        return (-1, 0);
    }
    *slot = Some(id);
    (crate::FD_PENDING, 0)
}
/// The answer of the picker file_dialog opened: None while it is open (or there is none),
/// then once (1, 0, the content:// name) or (0, 0, "") for cancelled / failed.
#[cfg(target_os = "android")]
pub fn file_dialog_poll() -> Option<(i32, u32, String)> {
    let mut slot = DIALOG.lock();
    let id = (*slot)?;
    let mut requests = REQUESTS.lock();
    let i = requests.iter().position(|r| r.id == id)?;
    if requests[i].status == RequestStatus::Pending {
        return None;
    }
    let r = requests.remove(i);
    *slot = None;
    Some(match (r.status, r.result) {
        (RequestStatus::Completed, Some(uri)) => (1, 0, uri),
        _ => (0, 0, String::new()),
    })
}
// (Android) the request of the open picker
#[cfg(target_os = "android")]
static DIALOG: SpinLock<Option<u32>> = SpinLock::new(None);
/// Native printing is disabled until a browser print adapter has been connected.
pub fn print(_job: &mut crate::PrintJob<'_>) -> i32 {
    crate::ERR_UNSUPPORTED
}
/// Operating-system folder launch is unavailable.
pub fn open_folder(_path: &str) -> i32 {
    -1
}
/// Request an explicitly selected web URL; the host still enforces user-gesture restrictions.
pub fn open_url(url: &str) -> i32 {
    if !["https://", "http://", "mailto:"]
        .iter()
        .any(|p| url.starts_with(p))
        || !crate::policy::url_allowed(url)
    {
        return -1;
    }
    if request(REQUEST_URL, url.as_bytes(), None).is_some() {
        0
    } else {
        -1
    }
}
/// Process spawning is unavailable in the browser.
pub fn spawn_self(_args: &[&str]) -> i32 {
    -1
}
/// Process spawning is unavailable in the browser.
pub fn restart_self(_args: &[&str]) -> i32 {
    -1
}
/// Number of pen devices is not exposed by browser pointer events.
pub fn pen_devices() -> i32 {
    -1
}

/// Update the DOM IME/composition caret position in physical pixels.
pub fn ime_pos(x: i32, y: i32, height: i32) {
    let mut bytes = [0; 12];
    for (i, value) in [x, y, height].iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    notice(REQUEST_IME, &bytes);
}
/// Set the local multi-format clipboard; browser system text writes are asynchronous requests.
/// Android: text and HTML (an image: its PNG) go to the system's clipboard at once; the richer
/// forms stay in the app and count while the system's clip is still this one.
pub fn clip_set_many(entries: &[(u32, &[u8])]) -> bool {
    if !crate::clip_many_valid(entries) {
        return false;
    }
    *CLIP.lock() = entries
        .iter()
        .map(|(format, bytes)| (*format, bytes.to_vec()))
        .collect();
    clip_publish(entries)
}
#[cfg(not(target_os = "android"))]
fn clip_publish(entries: &[(u32, &[u8])]) -> bool {
    if capabilities().supports(crate::CAP_CLIPBOARD_TEXT) {
        if let Some((_, bytes)) = entries
            .iter()
            .find(|(format, _)| *format == crate::CLIP_TEXT)
        {
            let _ = request(REQUEST_CLIP_WRITE, bytes, None);
        }
    }
    true
}
#[cfg(target_os = "android")]
fn clip_publish(entries: &[(u32, &[u8])]) -> bool {
    let get = |f: u32| entries.iter().find(|(g, _)| *g == f).map_or(&[][..], |(_, b)| *b);
    android_host::clip_publish(get(crate::CLIP_TEXT), get(crate::CLIP_HTML), &[])
}
/// Set one instance-local clipboard representation.  Android: CLIP_IMAGE (the record of
/// ui::clip_set_image) and CLIP_PNG go to the system's clipboard as a PNG.
pub fn clip_set(format: u32, bytes: &[u8]) -> bool {
    #[cfg(target_os = "android")]
    if matches!(format, crate::CLIP_IMAGE | crate::CLIP_PNG) {
        let png = if format == crate::CLIP_PNG { Some(bytes).filter(|b| b.starts_with(b"\x89PNG")) } else { image_png(bytes) };
        let Some(png) = png else { return false };
        *CLIP.lock() = alloc::vec![(format, bytes.to_vec())];
        return android_host::clip_publish(&[], &[], png);
    }
    clip_set_many(&[(format, bytes)])
}
/// Read an available clipboard representation; the host refreshes text before paste input.
/// Android: text, HTML and images from the system's clipboard; the app's own richer forms
/// while the system's clip is the one it set.
pub fn clip_get(format: u32) -> Option<Vec<u8>> {
    #[cfg(target_os = "android")]
    {
        if format == crate::CLIP_TEXT {
            return android_host::clip_read(format);
        }
        let own = clip_own();
        let local = || CLIP.lock().iter().find(|(f, _)| *f == format).map(|(_, v)| v.clone());
        return match format {
            crate::CLIP_IMAGE | crate::CLIP_PNG if own => {
                let l = CLIP.lock();
                let png = l.iter().find(|(f, _)| *f == crate::CLIP_PNG).map(|(_, v)| v.clone());
                png.or_else(|| l.iter().find(|(f, _)| *f == crate::CLIP_IMAGE).and_then(|(_, r)| image_png(r)).map(<[u8]>::to_vec))
            }
            crate::CLIP_IMAGE => android_host::clip_read(format),
            // PNG file bytes only
            crate::CLIP_PNG => android_host::clip_read(format).filter(|b| b.starts_with(b"\x89PNG")),
            crate::CLIP_HTML if !own => android_host::clip_read(format),
            _ if own => local(),
            _ => None,
        };
    }
    #[cfg(not(target_os = "android"))]
    CLIP.lock()
        .iter()
        .find(|(f, _)| *f == format)
        .map(|(_, v)| v.clone())
}
/// Whether an instance-local clipboard representation is present (Android: on the system's
/// clipboard, or the app's own form of its clip).
pub fn clip_has(format: u32) -> bool {
    #[cfg(target_os = "android")]
    {
        if matches!(format, crate::CLIP_TEXT | crate::CLIP_IMAGE | crate::CLIP_PNG) || !clip_own() {
            return android_host::clip_offers(format);
        }
    }
    CLIP.lock().iter().any(|(f, _)| *f == format)
}
// (Android) the system's clip is the one the app set last (CLIP holds its forms)
#[cfg(target_os = "android")]
fn clip_own() -> bool {
    let (text, image) = {
        let l = CLIP.lock();
        let text = l.iter().find(|(f, _)| *f == crate::CLIP_TEXT).map(|(_, v)| v.clone());
        (text, l.iter().any(|(f, _)| matches!(*f, crate::CLIP_IMAGE | crate::CLIP_PNG)))
    };
    android_host::clip_own(text.as_deref(), image)
}
// (Android) the PNG of a CLIP_IMAGE record (u64 length + PNG, u64 length + BMP)
#[cfg(target_os = "android")]
fn image_png(record: &[u8]) -> Option<&[u8]> {
    let n = usize::try_from(u64::from_le_bytes(record.get(..8)?.try_into().ok()?)).ok()?;
    record.get(8..8usize.checked_add(n)?).filter(|p| p.starts_with(b"\x89PNG"))
}

/// (dnd.rs) the browser and Android hosts do not drag out of the app (yet)
pub(crate) fn os_drag_start(_data: crate::DragData) -> bool {
    false
}

pub(crate) fn input(len: u32) -> Option<Vec<u8>> {
    INBOX.lock().get(..len as usize).map(|b| b.to_vec())
}
/// clear the host input buffer (it held secrets)
#[cfg(target_arch = "wasm32")]
pub(crate) fn wipe_input() {
    crypto::zeroize(&mut INBOX.lock());
}

/// Reserve the reusable host input buffer (at most 256 MiB); zero means allocation failure.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_buffer(len: u32) -> usize {
    if len > 256 << 20 {
        return 0;
    }
    let mut input = INBOX.lock();
    let extra = (len as usize).saturating_sub(input.len());
    if input.try_reserve(extra).is_err() {
        return 0;
    }
    input.resize(len as usize, 0);
    input.as_mut_ptr() as usize
}
/// Start once. Inbox contains newline-separated arguments, excluding the synthetic program name.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_start(width: i32, height: i32, dpi: i32, args_len: u32) -> i32 {
    if STARTED.swap(true, Ordering::Relaxed) {
        return -1;
    }
    let Some(bytes) = input(args_len) else {
        return -1;
    };
    let Ok(text) = core::str::from_utf8(&bytes) else {
        return -1;
    };
    let mut args = alloc::vec![String::from(if cfg!(target_os = "android") { "jfgui-android" } else { "jfgui-web" })];
    args.extend(
        text.lines()
            .filter(|v| !v.is_empty())
            .take(256)
            .map(String::from),
    );
    AGENT.store(args.iter().any(|a| a == "--agent"), Ordering::Relaxed);
    *ARGS.lock() = args;
    crate::cpu::simd_override();
    windows::viewport(width, height, dpi);
    rt::boot::set_handler(crate::boot::start); // rt::main!: name, app://, manifest
    // SAFETY: app entry is linked into this module, invoked only once on its worker.
    unsafe { jfgui_main() }
}
/// Advance the owned runtime and cooperative jobs, then submit the composed framebuffer.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_tick(time_ms: u32) {
    #[cfg(not(web_threads))]
    crate::pump_jobs(16);
    drain_wake();
    let _ = with_app(|app| app.tick(time_ms));
    windows::flush();
}
/// Drain input in order, deferring expensive rasterization until the batch's next tick.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_input(time_ms: u32) {
    drain_wake();
    let _ = with_app(|app| app.input(time_ms));
}
/// Idle host scheduling hint. The host must still wake immediately for new input.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_frame_delay() -> u32 {
    with_app(|app| app.frame_delay()).unwrap_or(250)
}
/// Enqueue a bounded platform event; pointer/resize events update virtual window routing.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_event(
    typ: u32,
    button: u32,
    mods: u32,
    window: u32,
    x: i32,
    y: i32,
    data: u32,
) -> u32 {
    if typ > crate::E_DROP as u32 || window >= crate::PW_MAX as u32 {
        return 0;
    }
    windows::event(Event {
        typ: typ as u8,
        btn: button as u8,
        mods: mods as u8,
        win: window as u8,
        x,
        y,
        data,
    }) as u32
}
/// Set a per-instance environment value from the host before jfgui_start; inbox: UTF-8
/// "NAME=value" (the host's languages arrive as LANGUAGE=de-DE:en-US).  1 = set.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_env(len: u32) -> u32 {
    let Some(bytes) = input(len) else {
        return 0;
    };
    let Some((name, value)) = core::str::from_utf8(&bytes).ok().and_then(|t| t.split_once('=')) else {
        return 0;
    };
    setenv(name, Some(value)) as u32
}
/// Enqueue committed UTF-8 from the host composition element; uncommitted IME text stays in the DOM.
/// The text goes as commits (whole strings, not one ring slot per character) -> the characters
/// queued: fewer when the queue is full (the host calls again with the rest after jfgui_input),
/// none without a window that takes text.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_text(len: u32) -> u32 {
    let Some(bytes) = input(len) else {
        return 0;
    };
    let Ok(text) = core::str::from_utf8(&bytes) else {
        return 0;
    };
    let Some(win) = windows::text_target() else {
        return 0;
    };
    let n = crate::ime_commit_text(win, text);
    text[..n].chars().count() as u32
}
/// Process one MessagePort/JavaScript agent request only after explicit activation.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_control(len: u32) -> u32 {
    OUTBOX.lock().clear();
    if !AGENT.load(Ordering::Relaxed) {
        return 0;
    }
    let Some(bytes) = input(len) else {
        return 0;
    };
    let Some(Some(response)) = with_app(|app| app.control(&bytes)) else {
        return 0;
    };
    *OUTBOX.lock() = response.into_bytes();
    OUTBOX.lock().len() as u32
}
/// Address of the current response, valid until the next control request.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_response() -> usize {
    OUTBOX.lock().as_ptr() as usize
}
/// Disable browser control immediately; reactivation must be explicit in application UI/startup.
pub fn set_agent_enabled(enabled: bool) {
    if AGENT.swap(enabled, Ordering::Relaxed) != enabled {
        notice(REQUEST_AGENT, &[u8::from(enabled)]);
    }
}

/// Explicitly enable or revoke browser automation after app startup.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_agent(enabled: u32) -> u32 {
    if !STARTED.load(Ordering::Relaxed) {
        return 0;
    }
    let enabled = enabled != 0;
    set_agent_enabled(enabled);
    with_app(|app| app.set_control_enabled(enabled));
    1
}
/// Grant only browser-supported dynamic services; native plugins/IPC/threads cannot be granted.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_capabilities(bits: u32) {
    let allowed = crate::CAP_PERSISTENT_STORAGE | crate::CAP_CLIPBOARD_TEXT;
    CAP_EXTRA.store(bits & allowed as u32, Ordering::Relaxed);
}
/// Refresh UTC calendar fields, validated before replacing the last host value.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_date(
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    millis: u32,
    weekday: u32,
) -> u32 {
    if year > 65535
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
        || millis > 999
        || weekday > 6
    {
        return 0;
    }
    *CALENDAR.lock() = Date {
        year: year as u16,
        month: month as u16,
        day: day as u16,
        hour: hour as u16,
        min: minute as u16,
        sec: second as u16,
        ms: millis as u16,
        wday: weekday as u16,
    };
    1
}
/// Import path + file bytes from the inbox. Selected files can be announced through E_DROP.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_import(path_len: u32, total_len: u32, dropped: u32, durable: u32) -> u32 {
    let Some(bytes) = input(total_len) else {
        return 0;
    };
    let Some(path_bytes) = bytes.get(..path_len as usize) else {
        return 0;
    };
    let Ok(path) = core::str::from_utf8(path_bytes) else {
        return 0;
    };
    if !write_file_raw(path, &bytes[path_len as usize..]) {
        return 0;
    }
    if durable != 0 {
        if let Some((path, revision, _)) = files::snapshot(path) {
            files::persisted(&path, revision);
        }
    }
    if dropped != 0 && crate::drop_add(path) {
        let _ = crate::evq_push(Event {
            typ: crate::E_DROP,
            data: 1,
            ..Event::default()
        });
    }
    1
}
/// Update a browser clipboard representation before delivering the corresponding paste key.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_clipboard(format: u32, len: u32) -> u32 {
    let Some(bytes) = input(len) else {
        return 0;
    };
    if format == 0 {
        // Atomic host import: count, then (format, byte length, UTF-8 bytes), little endian.
        // Validate every representation before replacing the previous clipboard.
        let mut at=0usize;
        let read=|at:&mut usize|->Option<u32>{let end=at.checked_add(4)?;let b=bytes.get(*at..end)?;*at=end;Some(u32::from_le_bytes(b.try_into().ok()?))};
        let Some(count)=read(&mut at).filter(|n|*n>0&&*n<=3) else{return 0;};
        let mut entries=Vec::new();
        for _ in 0..count {
            let Some(f)=read(&mut at).filter(|f|matches!(*f,crate::CLIP_TEXT|crate::CLIP_HTML|crate::CLIP_RTF)) else{return 0;};
            let Some(n)=read(&mut at) else{return 0;};let Some(end)=at.checked_add(n as usize) else{return 0;};
            let Some(data)=bytes.get(at..end) else{return 0;};at=end;entries.push((f,data));
        }
        if at!=bytes.len() || !crate::clip_many_valid(&entries){return 0;}
        let mut clip=CLIP.lock();
        // A browser which only exposes text can still paste our own richer local copy.
        // Any external rich payload, or changed text, replaces all cached representations.
        // Windows clipboard round-trips LF as CRLF; that is still our own text.
        fn canonical(bytes:&[u8])->impl Iterator<Item=u8>+'_ {
            bytes.iter().copied().enumerate().filter_map(move |(i,c)|
                if c==b'\r' && bytes.get(i+1)==Some(&b'\n'){None}else{Some(c)})
        }
        let same_text=entries.len()==1 && entries[0].0==crate::CLIP_TEXT && clip.iter().any(|(f,v)|*f==crate::CLIP_TEXT&&canonical(v).eq(canonical(entries[0].1)));
        if !same_text {*clip=entries.into_iter().map(|(f,v)|(f,v.to_vec())).collect();}
        return 1;
    }
    if !crate::clip_many_valid(&[(format, &bytes)]) {
        return 0;
    }
    let mut clip = CLIP.lock();
    clip.retain(|(f, _)| *f != format);
    clip.push((format, bytes));
    1
}
/// (Android) the answer of the system's picker: the inbox holds the chosen content:// URI
/// (status 2), a negative status as in jfgui_complete (-4 cancelled).  A file_dialog request
/// keeps the URI for file_dialog_poll; pick_file's becomes a drop (E_DROP).  1 = taken.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_file_result(id: u32, status: i32, len: u32) -> u32 {
    let uri = input(len).and_then(|b| String::from_utf8(b).ok()).filter(|u| u.starts_with("content://"));
    let kind = {
        let mut requests = REQUESTS.lock();
        let Some(r) = requests.iter_mut().find(|r| r.id == id && r.status == RequestStatus::Pending) else {
            return 0;
        };
        if status != 2 || uri.is_none() {
            drop(requests);
            return jfgui_complete(id, if status == 2 { -5 } else { status });
        }
        r.status = RequestStatus::Completed;
        r.result = uri.clone();
        r.kind
    };
    if kind == REQUEST_OPEN {
        if let Some(u) = &uri {
            if crate::drop_add(u) {
                let _ = crate::evq_push(Event { typ: crate::E_DROP, data: 1, ..Event::default() });
            }
        }
    }
    wake();
    1
}
/// Confirm a host operation: 0 durable, 1 exported, 2 completed; negatives are explicit errors.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_complete(id: u32, status: i32) -> u32 {
    let mut requests = REQUESTS.lock();
    let Some(request) = requests
        .iter_mut()
        .find(|r| r.id == id && r.status == RequestStatus::Pending)
    else {
        return 0;
    };
    request.status = match status {
        0 if request.kind == REQUEST_PERSIST => RequestStatus::Durable,
        1 if request.kind == REQUEST_DOWNLOAD => RequestStatus::Exported,
        2 if request.kind != REQUEST_PERSIST && request.kind != REQUEST_DOWNLOAD => {
            RequestStatus::Completed
        }
        -1 => RequestStatus::Denied,
        -2 => RequestStatus::Quota,
        -3 => RequestStatus::Unsupported,
        -4 => RequestStatus::Cancelled,
        _ => RequestStatus::Failed,
    };
    if request.status == RequestStatus::Durable {
        if let Some((path, revision)) = &request.file {
            files::persisted(path, *revision);
        }
    }
    let kind = request.kind;
    if request.detached {
        requests.retain(|request| request.id != id);
    }
    drop(requests);
    #[cfg(target_arch = "wasm32")]
    if kind == REQUEST_SECRET {
        crate::secret::committed(id, status);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = kind;
    wake();
    1
}
