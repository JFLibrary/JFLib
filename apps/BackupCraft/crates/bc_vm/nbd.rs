// nbd -- a read-only NBD server on a Unix socket (the fixed newstyle handshake, simple replies):
// QEMU reads the disks of a restore point through it, a qcow2 overlay on top takes the writes
//
//   serve(socket, exports) -> NbdServer       one thread accepts, one per connection;
//                                             dropping the server stops them all
//
// Options: EXPORT_NAME, GO and INFO (with the export's size and flags), LIST, ABORT; everything
// else is "unsupported" (structured replies, meta contexts).  Commands: READ, FLUSH (nothing
// to do), DISC; writes and trims are refused with EPERM (the export is read-only).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use plat::net::{UnixListener, UnixStream};

const NBDMAGIC: u64 = 0x4e42_444d_4147_4943;
const IHAVEOPT: u64 = 0x4948_4156_454f_5054;
const REPLY_MAGIC: u64 = 0x0003_e889_0455_65a9;
const REQUEST_MAGIC: u32 = 0x2560_9513;
const SIMPLE_REPLY: u32 = 0x6744_6698;

const FLAG_FIXED_NEWSTYLE: u16 = 1;
const FLAG_NO_ZEROES: u16 = 2;

const OPT_EXPORT_NAME: u32 = 1;
const OPT_ABORT: u32 = 2;
const OPT_LIST: u32 = 3;
const OPT_INFO: u32 = 6;
const OPT_GO: u32 = 7;

const REP_ACK: u32 = 1;
const REP_SERVER: u32 = 2;
const REP_INFO: u32 = 3;
const REP_ERR_UNSUP: u32 = 0x8000_0001;
const REP_ERR_INVALID: u32 = 0x8000_0003;
const REP_ERR_UNKNOWN: u32 = 0x8000_0006;

const INFO_EXPORT: u16 = 0;
const INFO_BLOCK_SIZE: u16 = 3;

const TFLAG_HAS_FLAGS: u16 = 1;
const TFLAG_READ_ONLY: u16 = 2;
const TFLAG_SEND_FLUSH: u16 = 4;
const TFLAG_CAN_MULTI_CONN: u16 = 1 << 8;

const CMD_READ: u16 = 0;
const CMD_DISC: u16 = 2;
const CMD_FLUSH: u16 = 3;

const EPERM: u32 = 1;
const EIO: u32 = 5;
const EINVAL: u32 = 22;

/// the largest read QEMU may ask for
const MAX_READ: u32 = 32 << 20;

/// reads `len` bytes at `offset` of an export
pub type ReadFn = Arc<dyn Fn(u64, usize) -> Result<Vec<u8>, String> + Send + Sync>;

#[derive(Clone)]
pub struct Export {
    pub name: String,
    pub size: u64,
    pub read: ReadFn,
}

pub struct NbdServer {
    stop: Arc<AtomicBool>,
    pub path: String,
}

impl Drop for NbdServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// serve the exports on a Unix socket until the server is dropped
pub fn serve(path: &str, exports: Vec<Export>) -> Result<NbdServer, String> {
    let l = UnixListener::bind(path).map_err(|e| alloc::format!("NBD {}: {}", path, e))?;
    let stop = Arc::new(AtomicBool::new(false));
    let (st, ex) = (stop.clone(), Arc::new(exports));
    plat::thread_spawn(64 * 1024, Box::new(move || {
        while !st.load(Ordering::SeqCst) {
            if let Ok(Some(c)) = l.accept_timeout(200) {
                let (st2, ex2) = (st.clone(), ex.clone());
                let _ = plat::thread_spawn(256 * 1024, Box::new(move || {
                    let _ = connection(&c, &ex2, &st2);
                }));
            }
        }
    })).ok_or_else(|| String::from("kein Thread für den NBD-Server"))?;
    Ok(NbdServer { stop, path: String::from(path) })
}

/// read exactly, waiting through timeouts until stopped
fn read_all(c: &UnixStream, b: &mut [u8], stop: &AtomicBool) -> Result<(), ()> {
    let mut done = 0;
    while done < b.len() {
        if stop.load(Ordering::SeqCst) {
            return Err(());
        }
        match c.read(&mut b[done..]) {
            Ok(0) => return Err(()),
            Ok(n) => done += n,
            Err(e) if e.kind == plat::net::NetErrorKind::TimedOut => {}
            Err(_) => return Err(()),
        }
    }
    Ok(())
}

fn send(c: &UnixStream, b: &[u8]) -> Result<(), ()> {
    c.write_all(b).map_err(|_| ())
}

fn reply(c: &UnixStream, opt: u32, ty: u32, data: &[u8]) -> Result<(), ()> {
    let mut b = Vec::with_capacity(20 + data.len());
    b.extend_from_slice(&REPLY_MAGIC.to_be_bytes());
    b.extend_from_slice(&opt.to_be_bytes());
    b.extend_from_slice(&ty.to_be_bytes());
    b.extend_from_slice(&(data.len() as u32).to_be_bytes());
    b.extend_from_slice(data);
    send(c, &b)
}

fn tflags() -> u16 {
    TFLAG_HAS_FLAGS | TFLAG_READ_ONLY | TFLAG_SEND_FLUSH | TFLAG_CAN_MULTI_CONN
}

fn connection(c: &UnixStream, exports: &[Export], stop: &AtomicBool) -> Result<(), ()> {
    c.set_read_timeout(Some(1000));
    c.set_write_timeout(Some(60_000));
    let mut hello = Vec::new();
    hello.extend_from_slice(&NBDMAGIC.to_be_bytes());
    hello.extend_from_slice(&IHAVEOPT.to_be_bytes());
    hello.extend_from_slice(&(FLAG_FIXED_NEWSTYLE | FLAG_NO_ZEROES).to_be_bytes());
    send(c, &hello)?;
    let mut cf = [0u8; 4];
    read_all(c, &mut cf, stop)?;
    let no_zeroes = u32::from_be_bytes(cf) & FLAG_NO_ZEROES as u32 != 0;
    // option haggling
    let export = loop {
        let mut h = [0u8; 16];
        read_all(c, &mut h, stop)?;
        let magic = u64::from_be_bytes([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]]);
        let opt = u32::from_be_bytes([h[8], h[9], h[10], h[11]]);
        let len = u32::from_be_bytes([h[12], h[13], h[14], h[15]]);
        if magic != IHAVEOPT || len > 65536 {
            return Err(());
        }
        let mut data = alloc::vec![0u8; len as usize];
        read_all(c, &mut data, stop)?;
        match opt {
            OPT_EXPORT_NAME => {
                let name = String::from_utf8_lossy(&data).into_owned();
                let Some(e) = exports.iter().find(|e| e.name == name || (name.is_empty() && exports.len() == 1)) else { return Err(()) };
                let mut b = Vec::new();
                b.extend_from_slice(&e.size.to_be_bytes());
                b.extend_from_slice(&tflags().to_be_bytes());
                if !no_zeroes {
                    b.extend_from_slice(&[0u8; 124]);
                }
                send(c, &b)?;
                break e.clone();
            }
            OPT_ABORT => {
                let _ = reply(c, opt, REP_ACK, &[]);
                return Ok(());
            }
            OPT_LIST => {
                for e in exports {
                    let mut d = Vec::new();
                    d.extend_from_slice(&(e.name.len() as u32).to_be_bytes());
                    d.extend_from_slice(e.name.as_bytes());
                    reply(c, opt, REP_SERVER, &d)?;
                }
                reply(c, opt, REP_ACK, &[])?;
            }
            OPT_INFO | OPT_GO => {
                if data.len() < 6 {
                    reply(c, opt, REP_ERR_INVALID, &[])?;
                    continue;
                }
                let nl = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                if data.len() < 4 + nl + 2 {
                    reply(c, opt, REP_ERR_INVALID, &[])?;
                    continue;
                }
                let name = String::from_utf8_lossy(&data[4..4 + nl]).into_owned();
                let Some(e) = exports.iter().find(|e| e.name == name || (name.is_empty() && exports.len() == 1)) else {
                    reply(c, opt, REP_ERR_UNKNOWN, &[])?;
                    continue;
                };
                let mut info = Vec::new();
                info.extend_from_slice(&INFO_EXPORT.to_be_bytes());
                info.extend_from_slice(&e.size.to_be_bytes());
                info.extend_from_slice(&tflags().to_be_bytes());
                reply(c, opt, REP_INFO, &info)?;
                let mut bs = Vec::new();
                bs.extend_from_slice(&INFO_BLOCK_SIZE.to_be_bytes());
                bs.extend_from_slice(&1u32.to_be_bytes());
                bs.extend_from_slice(&4096u32.to_be_bytes());
                bs.extend_from_slice(&MAX_READ.to_be_bytes());
                reply(c, opt, REP_INFO, &bs)?;
                reply(c, opt, REP_ACK, &[])?;
                if opt == OPT_GO {
                    break e.clone();
                }
            }
            _ => reply(c, opt, REP_ERR_UNSUP, &[])?,
        }
    };
    // transmission
    loop {
        let mut r = [0u8; 28];
        read_all(c, &mut r, stop)?;
        let magic = u32::from_be_bytes([r[0], r[1], r[2], r[3]]);
        let ty = u16::from_be_bytes([r[6], r[7]]);
        let handle = &r[8..16];
        let off = u64::from_be_bytes([r[16], r[17], r[18], r[19], r[20], r[21], r[22], r[23]]);
        let len = u32::from_be_bytes([r[24], r[25], r[26], r[27]]);
        if magic != REQUEST_MAGIC {
            return Err(());
        }
        let head = |err: u32| {
            let mut b = Vec::with_capacity(16);
            b.extend_from_slice(&SIMPLE_REPLY.to_be_bytes());
            b.extend_from_slice(&err.to_be_bytes());
            b.extend_from_slice(handle);
            b
        };
        match ty {
            CMD_READ => {
                if len > MAX_READ || off.checked_add(len as u64).is_none_or(|end| end > export.size) {
                    send(c, &head(EINVAL))?;
                    continue;
                }
                match (export.read)(off, len as usize) {
                    Ok(data) if data.len() == len as usize => {
                        let mut b = head(0);
                        b.extend_from_slice(&data);
                        send(c, &b)?;
                    }
                    _ => send(c, &head(EIO))?,
                }
            }
            CMD_DISC => return Ok(()),
            CMD_FLUSH => send(c, &head(0))?,
            1 => {
                // a write: take its data off the wire, refuse it
                let mut sink = alloc::vec![0u8; len.min(MAX_READ) as usize];
                read_all(c, &mut sink, stop)?;
                send(c, &head(EPERM))?;
            }
            _ => send(c, &head(EINVAL))?,
        }
    }
}
