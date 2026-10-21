// aio -- files without blocking the calling thread: the work runs on the blocking pool
// (task.rs) in chunks, with progress and cancellation between the chunks           (aio.rs)
//
//   read_file(path, &ctl) -> future of Result<Vec<u8>, IoError>
//   write_file(path, data, &ctl) -> future of Result<(), IoError>   atomic like write_file:
//                         AtomicFile (a sibling written in chunks, put in place by the commit);
//                         cancelled or failed, the sibling is removed and the old file stays
//   copy_file(src, dst, &ctl) -> future of Result<u64, IoError>     src read in chunks into an
//                         AtomicFile of dst -> the bytes copied
//   Reader::open(path) -> reader;  reader.read(max).await -> the next chunk ([] at the end),
//                         size() / pos() / seek(pos): a plat::File (OS file or a mount's stream)
//                         read chunk by chunk, each chunk a job of the pool
//   Writer::create(path) -> writer;  writer.write(data).await, writer.commit().await: an
//                         AtomicFile filled chunk by chunk (dropped without commit: discarded)
//   Ctl { progress, cancel, chunk }   what an operation reports to (bytes done / total) and
//                         listens to; clones share both
//
// Names, routing and permissions are those of the blocking functions (guarded.rs, vfs.rs): the
// same calls, made on a pool thread (a caller inside policy's trusted() stays trusted there).
// A cancel is seen between chunks: the operation cleans up and ends with Err(Cancelled).
// Dropping one of the futures cancels it the same way.  Where no thread can start (browser,
// Android) the pool's jobs run on the event loop's thread between frames.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;

use crate::task::{spawn_blocking, spawn_cancellable, CancelToken, Progress};
use crate::{AtomicFile, File, FO_READ};

/// the chunk of read_file / write_file / copy_file unless Ctl says otherwise
pub const CHUNK: usize = 256 * 1024;

/// why a file operation failed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoError {
    /// the source is missing, refused (permissions) or no file
    Open,
    /// the target (its sibling) could not be created: refused, a folder, no such folder
    Create,
    Read,
    Write,
    /// the finished file could not be put in place
    Commit,
    /// the CancelToken was cancelled (or the future dropped) before the end
    Cancelled,
}

impl core::fmt::Display for IoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            IoError::Open => "cannot open the file",
            IoError::Create => "cannot create the file",
            IoError::Read => "read error",
            IoError::Write => "write error",
            IoError::Commit => "cannot replace the file",
            IoError::Cancelled => "cancelled",
        })
    }
}

/// progress and cancellation of one operation (clones share them)
#[derive(Clone)]
pub struct Ctl {
    /// bytes done and the total (set when the size is known)
    pub progress: Progress,
    /// cancel() stops the operation at the next chunk
    pub cancel: CancelToken,
    /// bytes per chunk (0: CHUNK)
    pub chunk: usize,
}

impl Ctl {
    /// a fresh Progress and CancelToken, CHUNK bytes per chunk
    pub fn new() -> Ctl {
        Ctl { progress: Progress::new(), cancel: CancelToken::new(), chunk: CHUNK }
    }

    fn chunk(&self) -> usize {
        if self.chunk == 0 { CHUNK } else { self.chunk }
    }

    // Err(Cancelled) when the caller cancelled or nobody waits any more
    fn stop(&self, gone: &CancelToken) -> Result<(), IoError> {
        if self.cancel.is_cancelled() || gone.is_cancelled() { Err(IoError::Cancelled) } else { Ok(()) }
    }
}

impl Default for Ctl {
    fn default() -> Self {
        Ctl::new()
    }
}

/// the contents of a file, read in chunks on the pool
pub fn read_file(path: &str, ctl: &Ctl) -> impl Future<Output = Result<Vec<u8>, IoError>> + use<> {
    let (path, ctl) = (String::from(path), ctl.clone());
    spawn_cancellable(move |gone| {
        ctl.stop(gone)?;
        let f = File::open(&path, FO_READ).ok_or(IoError::Open)?;
        let size = f.size().ok_or(IoError::Read)?;
        ctl.progress.set_total(size);
        let mut out = Vec::new();
        out.try_reserve(usize::try_from(size).map_err(|_| IoError::Read)?).map_err(|_| IoError::Read)?;
        let mut buf = alloc::vec![0u8; ctl.chunk()];
        loop {
            ctl.stop(gone)?;
            let n = f.read_at(out.len() as u64, &mut buf).ok_or(IoError::Read)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            ctl.progress.set(out.len() as u64);
        }
        Ok(out)
    })
}

/// write data in chunks into an AtomicFile of path, then put it in place
pub fn write_file(path: &str, data: Vec<u8>, ctl: &Ctl) -> impl Future<Output = Result<(), IoError>> + use<> {
    let (path, ctl) = (String::from(path), ctl.clone());
    spawn_cancellable(move |gone| {
        ctl.stop(gone)?;
        let w = AtomicFile::create(&path).ok_or(IoError::Create)?;
        ctl.progress.set_total(data.len() as u64);
        let mut off = 0;
        while off < data.len() {
            ctl.stop(gone)?; // w is dropped: the sibling goes
            let n = (data.len() - off).min(ctl.chunk());
            if w.write_at(off as u64, &data[off..off + n]) != Some(n) {
                return Err(IoError::Write);
            }
            off += n;
            ctl.progress.set(off as u64);
        }
        ctl.stop(gone)?;
        if w.commit() { Ok(()) } else { Err(IoError::Commit) }
    })
}

/// copy src to dst (atomic like write_file) in chunks -> the bytes copied
pub fn copy_file(src: &str, dst: &str, ctl: &Ctl) -> impl Future<Output = Result<u64, IoError>> + use<> {
    let (src, dst, ctl) = (String::from(src), String::from(dst), ctl.clone());
    spawn_cancellable(move |gone| {
        ctl.stop(gone)?;
        let f = File::open(&src, FO_READ).ok_or(IoError::Open)?;
        ctl.progress.set_total(f.size().ok_or(IoError::Read)?);
        let w = AtomicFile::create(&dst).ok_or(IoError::Create)?;
        let mut buf = alloc::vec![0u8; ctl.chunk()];
        let mut off = 0u64;
        loop {
            ctl.stop(gone)?;
            let n = f.read_at(off, &mut buf).ok_or(IoError::Read)?;
            if n == 0 {
                break;
            }
            if w.write_at(off, &buf[..n]) != Some(n) {
                return Err(IoError::Write);
            }
            off += n as u64;
            ctl.progress.set(off);
        }
        drop(f);
        ctl.stop(gone)?;
        if w.commit() { Ok(off) } else { Err(IoError::Commit) }
    })
}

/// a file read chunk by chunk on the pool
pub struct Reader {
    f: Arc<File>,
    size: u64,
    pos: u64,
}

impl Reader {
    /// open path for reading (on the pool)
    pub fn open(path: &str) -> impl Future<Output = Result<Reader, IoError>> + use<> {
        let path = String::from(path);
        spawn_blocking(move || {
            let f = File::open(&path, FO_READ).ok_or(IoError::Open)?;
            let size = f.size().ok_or(IoError::Read)?;
            Ok(Reader { f: Arc::new(f), size, pos: 0 })
        })
    }

    /// the size when it was opened
    pub fn size(&self) -> u64 {
        self.size
    }

    /// where the next read starts
    pub fn pos(&self) -> u64 {
        self.pos
    }

    /// the next read starts at pos
    pub fn seek(&mut self, pos: u64) {
        self.pos = pos;
    }

    /// the next chunk of at most max bytes (empty at the end of the file)
    pub async fn read(&mut self, max: usize) -> Result<Vec<u8>, IoError> {
        let (f, off) = (self.f.clone(), self.pos);
        let chunk = spawn_blocking(move || {
            let mut b = alloc::vec![0u8; max];
            let n = f.read_at(off, &mut b)?;
            b.truncate(n);
            Some(b)
        })
        .await
        .ok_or(IoError::Read)?;
        self.pos += chunk.len() as u64;
        Ok(chunk)
    }
}

/// an AtomicFile filled chunk by chunk on the pool; dropped without commit, it is discarded
/// (the sibling is removed by whoever drops the last reference: the dropping thread, or the
/// pool when a write was still running)
pub struct Writer {
    f: Option<Arc<AtomicFile>>,
    pos: u64,
}

impl Writer {
    /// a writer of path (checked and routed like write_file; on the pool)
    pub fn create(path: &str) -> impl Future<Output = Result<Writer, IoError>> + use<> {
        let path = String::from(path);
        spawn_blocking(move || {
            let f = AtomicFile::create(&path).ok_or(IoError::Create)?;
            Ok(Writer { f: Some(Arc::new(f)), pos: 0 })
        })
    }

    /// bytes written so far
    pub fn written(&self) -> u64 {
        self.pos
    }

    /// append data (on the pool)
    pub async fn write(&mut self, data: Vec<u8>) -> Result<(), IoError> {
        let f = self.f.clone().ok_or(IoError::Write)?;
        let off = self.pos;
        let n = data.len();
        let ok = spawn_blocking(move || f.write_at(off, &data) == Some(data.len())).await;
        if !ok {
            return Err(IoError::Write);
        }
        self.pos += n as u64;
        Ok(())
    }

    /// put the written file in place (on the pool)
    pub async fn commit(mut self) -> Result<(), IoError> {
        let f = self.f.take().ok_or(IoError::Commit)?;
        let f = Arc::try_unwrap(f).map_err(|_| IoError::Commit)?; // a write still running
        if spawn_blocking(move || f.commit()).await { Ok(()) } else { Err(IoError::Commit) }
    }
}
