// android_host -- what Android's Java host does for plat while Rust waits: the
// ContentResolver behind content:// (content.rs: ContentFs over AndroidDocs), the system
// clipboard (ClipboardManager), the persisted grants of the user's documents.
//
// Rust asks synchronously on the UI thread: jfgui_host_call(kind, bytes) in bridge.c calls
// GalleryHost.call(kind, byte[]) (android/GalleryHost.java) and copies its answer into the
// buffer jfgui_reply hands out; null / an exception = None.  The file picker and the export of
// download() answer later (web.rs: REQUEST_FILE_DIALOG / REQUEST_OPEN / REQUEST_DOWNLOAD,
// jfgui_file_result).
//
// Wire (little endian): u32 numbers, i64 times, strings and byte blocks as u32 length + bytes.
//   CALL_QUERY     uri -> DocInfo, or nothing (missing / not allowed)
//   CALL_CHILDREN  uri -> u32 count, DocInfo..
//   CALL_CREATE    u32 dir, parent, name -> DocInfo
//   CALL_DELETE    uri -> u32 ok
//   CALL_RENAME    uri, name -> DocInfo
//   CALL_OPEN      u32 mode (FO_*), uri -> i32 file descriptor (detached: ours to close)
//   CALL_CLIP_GET  u32 format (CLIP_TEXT / CLIP_HTML / CLIP_IMAGE) -> u32 found, bytes
//   CALL_CLIP_SET  text, html, png (each may be empty) -> i64 stamp of the new clip
//   CALL_CLIP_HAS  u32 format -> u32
//   CALL_CLIP_STAMP -> i64 (ClipDescription.getTimestamp of the current clip, 0 = none or no
//                  access: Android lets only the focused app read the clipboard)
//   CALL_CLIP_OURS -> u32 (the clip's first item is a file of the app's GalleryFiles)
//   CALL_GRANTS    -> u32 count, (uri, u32 read, u32 write)..
//   CALL_RELEASE   uri -> u32 ok
//   DocInfo: uri, display name, u32 dir, u64 size (u64::MAX unknown), i64 mtime ms, u32 DOC_*
//
// The clipboard: text (and HTML) go to the system; the library's richer forms (RTF, DOCX)
// stay in the process and count while the system's clip is still the one we set (its
// timestamp; when the app had no focus while setting it -- right after the picker -- the
// contents decide once it can read them); images go as a PNG through the app's GalleryFiles
// provider when the manifest has one (else only inside the app).

use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicI64, Ordering};
use rt::sync::SpinLock;

use crate::content::{ContentFs, DocHost, DocInfo};
use crate::vfs::{Provider, Stream};
use crate::{CLIP_HTML, CLIP_IMAGE, CLIP_PNG, CLIP_TEXT, FO_READ, FO_RW, FO_WRITE};

const CALL_QUERY: u32 = 1;
const CALL_CHILDREN: u32 = 2;
const CALL_CREATE: u32 = 3;
const CALL_DELETE: u32 = 4;
const CALL_RENAME: u32 = 5;
const CALL_OPEN: u32 = 6;
const CALL_CLIP_GET: u32 = 10;
const CALL_CLIP_SET: u32 = 11;
const CALL_CLIP_HAS: u32 = 12;
const CALL_CLIP_STAMP: u32 = 13;
const CALL_CLIP_OURS: u32 = 14;
const CALL_GRANTS: u32 = 20;
const CALL_RELEASE: u32 = 21;

unsafe extern "C" {
    fn jfgui_host_call(kind: u32, ptr: *const u8, len: usize) -> i32;
}
#[link(name = "c")]
unsafe extern "C" {
    fn close(fd: i32) -> i32;
    fn read(fd: i32, buf: *mut u8, n: usize) -> isize;
    fn write(fd: i32, buf: *const u8, n: usize) -> isize;
    fn pread(fd: i32, buf: *mut u8, n: usize, off: i64) -> isize;
    fn pwrite(fd: i32, buf: *const u8, n: usize, off: i64) -> isize;
    fn lseek(fd: i32, off: i64, whence: i32) -> i64;
    fn fsync(fd: i32) -> i32;
    fn ftruncate(fd: i32, len: i64) -> i32;
}

static REPLY: SpinLock<Vec<u8>> = SpinLock::new(Vec::new());

/// (bridge.c) a buffer of len bytes for the answer of a host call; 0 = no memory
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_reply(len: u32) -> usize {
    let mut r = REPLY.lock();
    r.clear();
    if r.try_reserve_exact(len as usize).is_err() {
        return 0;
    }
    r.resize(len as usize, 0);
    r.as_mut_ptr() as usize
}

// one synchronous question to the Java host (the documents: only with fs.user, policy.rs)
fn call(kind: u32, payload: &[u8]) -> Option<Vec<u8>> {
    if (CALL_QUERY..=CALL_OPEN).contains(&kind) {
        let uri = core::str::from_utf8(payload).ok().filter(|u| u.starts_with("content:")).unwrap_or("content://");
        if !crate::policy::user_doc_allowed(uri) {
            return None;
        }
    }
    REPLY.lock().clear();
    // SAFETY: payload is copied by the host before it returns; the answer arrives through
    // jfgui_reply on this thread (no lock is held across the call).
    let n = unsafe { jfgui_host_call(kind, payload.as_ptr(), payload.len()) };
    let mut r = core::mem::take(&mut *REPLY.lock());
    let n = usize::try_from(n).ok()?;
    r.truncate(n);
    (r.len() == n).then_some(r)
}

// ---------------------------------------------------------------- wire

#[derive(Default)]
struct W(Vec<u8>);
impl W {
    fn u32(mut self, v: u32) -> W {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(mut self, b: &[u8]) -> W {
        self.0.extend_from_slice(&(b.len() as u32).to_le_bytes());
        self.0.extend_from_slice(b);
        self
    }
    fn str(self, s: &str) -> W {
        self.bytes(s.as_bytes())
    }
}

struct R<'a>(&'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (a, b) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = b;
        Some(a)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> Option<String> {
        Some(String::from(core::str::from_utf8(self.bytes()?).ok()?))
    }
    fn info(&mut self) -> Option<DocInfo> {
        let uri = self.str()?;
        let name = self.str()?;
        let dir = self.u32()? != 0;
        let size = self.u64()?;
        let mtime_ms = self.u64()? as i64;
        let flags = self.u32()?;
        Some(DocInfo { uri, name, dir, size: (size != u64::MAX).then_some(size), mtime_ns: mtime_ms.saturating_mul(1_000_000), flags })
    }
}

// ---------------------------------------------------------------- content://

/// the ContentResolver as ContentFs's host
pub struct AndroidDocs;

impl DocHost for AndroidDocs {
    fn query(&self, uri: &str) -> Option<DocInfo> {
        R(&call(CALL_QUERY, uri.as_bytes())?).info()
    }
    fn children(&self, uri: &str) -> Option<Vec<DocInfo>> {
        let a = call(CALL_CHILDREN, uri.as_bytes())?;
        let mut r = R(&a);
        let n = r.u32()?;
        (0..n).map(|_| r.info()).collect()
    }
    fn create(&self, parent: &str, name: &str, dir: bool) -> Option<DocInfo> {
        R(&call(CALL_CREATE, &W::default().u32(dir as u32).str(parent).str(name).0)?).info()
    }
    fn delete(&self, uri: &str) -> bool {
        call(CALL_DELETE, uri.as_bytes()).is_some_and(|a| R(&a).u32() == Some(1))
    }
    fn rename(&self, uri: &str, name: &str) -> Option<DocInfo> {
        R(&call(CALL_RENAME, &W::default().str(uri).str(name).0)?).info()
    }
    fn open(&self, uri: &str, mode: u32) -> Option<Box<dyn Stream>> {
        let a = call(CALL_OPEN, &W::default().u32(mode).str(uri).0)?;
        let fd = R(&a).u32()? as i32;
        (fd >= 0).then(|| FdStream::new(fd, mode)).flatten().map(|s| Box::new(s) as Box<dyn Stream>)
    }
}

/// the provider of content:// (mounted by vfs::route on first use)
pub fn content_fs() -> Arc<dyn Provider> {
    Arc::new(ContentFs::new(AndroidDocs))
}

/// the documents and folders the app keeps access to across restarts (the system's
/// persisted grants): (content:// URI, read, write)
pub fn content_grants() -> Vec<(String, bool, bool)> {
    let Some(a) = call(CALL_GRANTS, &[]) else { return Vec::new() };
    let mut r = R(&a);
    let Some(n) = r.u32() else { return Vec::new() };
    let mut out = Vec::new();
    for _ in 0..n {
        let (Some(u), Some(rd), Some(wr)) = (r.str(), r.u32(), r.u32()) else { break };
        out.push((u, rd != 0, wr != 0));
    }
    out
}

/// give up the lasting access to a document or folder the user chose (Android keeps at most
/// 128 / 512 of them per app) -> whether there was one
pub fn content_release(uri: &str) -> bool {
    call(CALL_RELEASE, uri.as_bytes()).is_some_and(|a| R(&a).u32() == Some(1))
}

/// a document's file descriptor: positional reads and writes; a pipe (a provider that
/// streams) read whole at the start, or written in order
struct FdStream {
    fd: i32,
    writable: bool,
    // a pipe: the bytes read at the start, or how many were written in order
    pipe: Option<SpinLock<(Vec<u8>, u64)>>,
}

impl FdStream {
    fn new(fd: i32, mode: u32) -> Option<FdStream> {
        let writable = matches!(mode, FO_WRITE | FO_RW);
        // SAFETY: fd is ours; a pipe answers ESPIPE
        if unsafe { lseek(fd, 0, 1) } >= 0 {
            return Some(FdStream { fd, writable, pipe: None });
        }
        if mode != FO_READ {
            return Some(FdStream { fd, writable, pipe: Some(SpinLock::new((Vec::new(), 0))) });
        }
        let mut all = Vec::new();
        let mut buf = alloc::vec![0u8; 64 * 1024];
        loop {
            // SAFETY: buf is writable; fd is open
            let n = unsafe { read(fd, buf.as_mut_ptr(), buf.len()) };
            if n < 0 {
                // SAFETY: fd is ours, closed once
                unsafe { close(fd) };
                return None;
            }
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n as usize]);
        }
        Some(FdStream { fd, writable, pipe: Some(SpinLock::new((all, 0))) })
    }
}

impl Stream for FdStream {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        if let Some(p) = &self.pipe {
            let g = p.lock();
            let off = usize::try_from(off).unwrap_or(usize::MAX).min(g.0.len());
            let n = (g.0.len() - off).min(buf.len());
            buf[..n].copy_from_slice(&g.0[off..off + n]);
            return Some(n);
        }
        // SAFETY: buf is writable; fd is open
        let n = unsafe { pread(self.fd, buf.as_mut_ptr(), buf.len(), i64::try_from(off).ok()?) };
        usize::try_from(n).ok()
    }
    fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        if !self.writable {
            return None;
        }
        let mut done = 0;
        if let Some(p) = &self.pipe {
            let mut g = p.lock();
            if off != g.1 {
                return None; // a pipe takes the bytes in order only
            }
            while done < buf.len() {
                // SAFETY: the rest of buf is readable; fd is open
                let n = unsafe { write(self.fd, buf[done..].as_ptr(), buf.len() - done) };
                if n <= 0 {
                    break;
                }
                done += n as usize;
            }
            g.1 += done as u64;
            return (done > 0 || buf.is_empty()).then_some(done);
        }
        let off = i64::try_from(off).ok()?;
        while done < buf.len() {
            // SAFETY: the rest of buf is readable; fd is open
            let n = unsafe { pwrite(self.fd, buf[done..].as_ptr(), buf.len() - done, off + done as i64) };
            if n <= 0 {
                return (done > 0).then_some(done);
            }
            done += n as usize;
        }
        Some(done)
    }
    fn size(&self) -> Option<u64> {
        if let Some(p) = &self.pipe {
            let g = p.lock();
            return Some(if self.writable { g.1 } else { g.0.len() as u64 });
        }
        // SAFETY: fd is open; reads and writes are positional
        u64::try_from(unsafe { lseek(self.fd, 0, 2) }).ok()
    }
    fn set_len(&self, len: u64) -> bool {
        // SAFETY: fd is open
        self.writable && self.pipe.is_none() && i64::try_from(len).is_ok_and(|l| unsafe { ftruncate(self.fd, l) } == 0)
    }
    fn sync(&self) -> bool {
        // SAFETY: fd is open (a pipe has nothing to sync)
        !self.writable || self.pipe.is_some() || unsafe { fsync(self.fd) } == 0
    }
}

impl Drop for FdStream {
    fn drop(&mut self) {
        // SAFETY: the descriptor is ours (detached from Java's ParcelFileDescriptor), closed once
        unsafe { close(self.fd) };
    }
}

// ---------------------------------------------------------------- the clipboard

// the timestamp of the clip we set last (0: none): the richer forms in web.rs's CLIP count
// while the system's clip is still that one
static OWN: AtomicI64 = AtomicI64::new(0);

fn stamp() -> i64 {
    call(CALL_CLIP_STAMP, &[]).and_then(|a| R(&a).u64()).map_or(0, |v| v as i64)
}

/// whether the system's clip is the one the app set last (its local forms count): the same
/// timestamp, or -- when the app could not read it while setting the clip -- the same text
/// (local_text), our own image file (local_image)
pub(super) fn clip_own(local_text: Option<&[u8]>, local_image: bool) -> bool {
    let own = OWN.load(Ordering::Relaxed);
    let now = stamp();
    if now == 0 {
        return false; // no clip, or no focus: nothing to compare
    }
    if own != 0 {
        return now == own;
    }
    // CRLF of other apps is still our text
    fn canon(b: &[u8]) -> impl Iterator<Item = u8> + '_ {
        b.iter().copied().enumerate().filter_map(move |(i, c)| (c != b'\r' || b.get(i + 1) != Some(&b'\n')).then_some(c))
    }
    let ours = match local_text {
        Some(t) => clip_read(CLIP_TEXT).is_some_and(|s| canon(&s).eq(canon(t))),
        None => local_image && call(CALL_CLIP_OURS, &[]).is_some_and(|a| R(&a).u32() == Some(1)),
    };
    if ours {
        OWN.store(now, Ordering::Relaxed);
    }
    ours
}

/// put text / HTML / a PNG on the system clipboard (empty = none of it)
pub(super) fn clip_publish(text: &[u8], html: &[u8], png: &[u8]) -> bool {
    let w = W::default().bytes(text).bytes(html).bytes(png);
    match call(CALL_CLIP_SET, &w.0).and_then(|a| R(&a).u64()) {
        Some(s) => {
            OWN.store(s as i64, Ordering::Relaxed);
            true
        }
        None => {
            OWN.store(0, Ordering::Relaxed);
            false
        }
    }
}

/// a form of the system's clip: text, HTML, an image (PNG / JPEG ... file bytes)
pub(super) fn clip_read(format: u32) -> Option<Vec<u8>> {
    let f = match format {
        CLIP_TEXT | CLIP_HTML => format,
        CLIP_IMAGE | CLIP_PNG => CLIP_IMAGE,
        _ => return None,
    };
    let a = call(CALL_CLIP_GET, &f.to_le_bytes())?;
    let mut r = R(&a);
    (r.u32()? != 0).then(|| r.bytes().map(<[u8]>::to_vec)).flatten()
}

/// whether the system's clip has text, HTML or an image
pub(super) fn clip_offers(format: u32) -> bool {
    let f = match format {
        CLIP_TEXT | CLIP_HTML => format,
        CLIP_IMAGE | CLIP_PNG => CLIP_IMAGE,
        _ => return false,
    };
    call(CALL_CLIP_HAS, &f.to_le_bytes()).is_some_and(|a| R(&a).u32() == Some(1))
}
