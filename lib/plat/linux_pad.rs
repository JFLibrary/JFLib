// linux_pad -- (G7a part 2) game pads on Linux: evdev (the shared part and the parser: pad.rs)
//
//   start() -> bool             an inotify watch on /dev/input, then every event node looked at
//   stop()                      every pad goes, the nodes and the watch are closed
//   poll(now, focused, full) -> next   read what the pads' nodes have (EvdevPad::feed, pad_update),
//                               every 250 ms what inotify reported (new nodes, nodes whose rights
//                               changed: udev sets them after the node appears); a node that says
//                               ENODEV or ends is a pad that went
//   evdev_test_attach(..)       (tests) a pad whose node is a pipe the test writes into
//
// A node is a pad when evdev_is_pad says so of its bitmaps (EVIOCGBIT): keys with BTN_SOUTH,
// absolute axes with ABS_X and ABS_Y.  udev's uaccess rule gives the user of the seat its game
// controllers (ID_INPUT_JOYSTICK), never keyboards or mice: what cannot be opened is no pad of
// ours, and is not opened again until inotify reports it anew.  Nodes are opened read-only and
// without blocking; their events are read whenever the pads are polled (4 ms), not from the window
// loop's poll().  Inside a Flatpak /dev/input exists only with --device=input (Flatpak 1.15.6 and
// newer); without it no pad is found.

use alloc::string::String;
use alloc::vec::Vec;

use rt::os::{AT_FDCWD, O_CLOEXEC, O_NONBLOCK, O_RDONLY, SYS_CLOSE, SYS_INOTIFY_ADD_WATCH, SYS_INOTIFY_INIT1, SYS_IOCTL, SYS_OPENAT, SYS_READ, syscall3, syscall6};
use rt::sync::SpinLock;

use crate::{AbsInfo, EVDEV_ABS, EVDEV_ABS_MAPPED, EVDEV_EVENT_SIZE, EVDEV_KEYS, EvdevPad, PAD_MAX, PadState, evdev_is_pad, pad_connect, pad_disconnect, pad_style, pad_update};

const EAGAIN: isize = 11;
const EINTR: isize = 4;
const IN_NONBLOCK: usize = 0o4000;
const IN_CLOEXEC: usize = 0o2000000;
const IN_ATTRIB: u32 = 0x4;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;
const IN_MOVED_TO: u32 = 0x80;
const POLL_US: u64 = 4_000;
const SCAN_US: u64 = 250_000;
/// without inotify: a look at the folder this often
const RESCAN_US: u64 = 2_000_000;

/// _IOC(_IOC_READ, 'E', nr, size): the same on x86-64 and ARM64
const fn eviocr(nr: usize, size: usize) -> usize {
    (2 << 30) | (size << 16) | ((b'E' as usize) << 8) | nr
}
const fn eviocgbit(ev: usize, len: usize) -> usize {
    eviocr(0x20 + ev, len)
}
const EVIOCGID: usize = eviocr(0x02, 8);
const fn eviocgname(len: usize) -> usize {
    eviocr(0x06, len)
}
const fn eviocgkey(len: usize) -> usize {
    eviocr(0x18, len)
}
const fn eviocgabs(abs: usize) -> usize {
    eviocr(0x40 + abs, 24)
}

struct Dev {
    fd: usize,
    /// N of /dev/input/eventN (u32::MAX: a test's pipe)
    node: u32,
    slot: usize,
    pad: EvdevPad,
}

struct Lp {
    /// the inotify descriptor (-1: none)
    ino: isize,
    devs: Vec<Dev>,
    /// nodes that are no pads (or could not be opened), until inotify reports them anew
    skip: Vec<u32>,
    /// a look at the folder is due
    rescan: bool,
    next_scan: u64,
    focused: bool,
}

static LP: SpinLock<Lp> = SpinLock::new(Lp { ino: -1, devs: Vec::new(), skip: Vec::new(), rescan: false, next_scan: 0, focused: false });

fn close(fd: usize) {
    // SAFETY: our descriptor.
    unsafe { syscall3(SYS_CLOSE, fd, 0, 0) };
}

fn ioctl(fd: usize, req: usize, buf: &mut [u8]) -> bool {
    // SAFETY: buf is writable for the size encoded in req (the callers pass a buffer of that size).
    unsafe { syscall3(SYS_IOCTL, fd, req, buf.as_mut_ptr() as usize) >= 0 }
}

/// the absolute axes a pad may map, as the kernel describes them
fn abs_infos(fd: usize, bits: &[u8]) -> Vec<AbsInfo> {
    let mut v = Vec::new();
    for code in 0..EVDEV_ABS_MAPPED {
        if bits[code / 8] & (1 << (code % 8)) == 0 {
            continue;
        }
        let mut a = [0u8; 24]; // struct input_absinfo: value, minimum, maximum, fuzz, flat, resolution
        if ioctl(fd, eviocgabs(code), &mut a) {
            let i = |o: usize| i32::from_ne_bytes([a[o], a[o + 1], a[o + 2], a[o + 3]]);
            v.push(AbsInfo { code: code as u16, value: i(0), min: i(4), max: i(8) });
        }
    }
    v
}

/// open /dev/input/eventN -> the pad on it (None: not a pad, or not ours to read)
fn open_node(n: u32, slot: usize) -> Option<(Dev, String, u16)> {
    let path = alloc::format!("/dev/input/event{n}\0");
    // SAFETY: a NUL-terminated path.
    let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, path.as_ptr() as usize, O_RDONLY | O_NONBLOCK | O_CLOEXEC, 0, 0, 0) };
    if fd < 0 {
        return None;
    }
    let fd = fd as usize;
    let (mut types, mut keys, mut abs) = ([0u8; 4], [0u8; EVDEV_KEYS / 8], [0u8; EVDEV_ABS / 8]);
    let ok = ioctl(fd, eviocgbit(0, types.len()), &mut types) && ioctl(fd, eviocgbit(1, keys.len()), &mut keys) && ioctl(fd, eviocgbit(3, abs.len()), &mut abs);
    if !ok || !evdev_is_pad(&types, &keys, &abs) {
        close(fd);
        return None;
    }
    let mut id = [0u8; 8]; // struct input_id: bustype, vendor, product, version
    let vendor = if ioctl(fd, EVIOCGID, &mut id) { u16::from_ne_bytes([id[2], id[3]]) } else { 0 };
    let mut nm = [0u8; 128];
    let name = if ioctl(fd, eviocgname(nm.len()), &mut nm) {
        let end = nm.iter().position(|&c| c == 0).unwrap_or(nm.len());
        String::from_utf8_lossy(&nm[..end]).into_owned()
    } else {
        String::from("Gamepad")
    };
    let infos = abs_infos(fd, &abs);
    let mut pad = EvdevPad::new(vendor, &keys, &infos);
    let mut held = [0u8; EVDEV_KEYS / 8];
    if ioctl(fd, eviocgkey(held.len()), &mut held) {
        pad.resync(&held, &infos);
    }
    Some((Dev { fd, node: n, slot, pad }, name, vendor))
}

/// the free slot for a new pad (None: PAD_MAX pads already)
fn free_slot(l: &Lp) -> Option<usize> {
    (0..PAD_MAX).find(|s| !l.devs.iter().any(|d| d.slot == *s))
}

/// look at every node of /dev/input not open and not skipped
fn scan(l: &mut Lp, focused: bool) {
    let mut nodes: Vec<u32> = Vec::new();
    super::sys::dir_list_raw("/dev/input", &mut |name, is_dir, _| {
        if let Some(n) = name.strip_prefix("event").and_then(|d| d.parse::<u32>().ok()) {
            if !is_dir {
                nodes.push(n);
            }
        }
    });
    nodes.sort_unstable();
    for n in nodes {
        if l.devs.iter().any(|d| d.node == n) || l.skip.contains(&n) {
            continue;
        }
        let Some(slot) = free_slot(l) else { return };
        match open_node(n, slot) {
            Some((dev, name, vendor)) => {
                pad_connect(slot, &name, pad_style(vendor, &name), true);
                pad_update(slot, &if focused { dev.pad.state() } else { PadState::NEUTRAL });
                l.devs.push(dev);
            }
            None => l.skip.push(n),
        }
    }
}

/// what inotify reported: nodes that came or changed their rights are looked at again
fn inotify_read(l: &mut Lp) {
    let mut buf = [0u8; 4096];
    loop {
        // SAFETY: the buffer is writable for its length.
        let n = unsafe { syscall3(SYS_READ, l.ino as usize, buf.as_mut_ptr() as usize, buf.len()) };
        if n == -EINTR {
            continue;
        }
        if n <= 0 {
            return;
        }
        let (n, mut off) = (n as usize, 0usize);
        while off + 16 <= n {
            let rd = |i: usize| u32::from_ne_bytes([buf[off + i], buf[off + i + 1], buf[off + i + 2], buf[off + i + 3]]);
            let (mask, len) = (rd(4), rd(12) as usize);
            let name = &buf[off + 16..(off + 16 + len).min(n)];
            let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
            off += 16 + len;
            let Some(node) = core::str::from_utf8(name).ok().and_then(|s| s.strip_prefix("event")).and_then(|d| d.parse::<u32>().ok()) else { continue };
            l.skip.retain(|&s| s != node);
            // (a pad's node going says ENODEV to its read: IN_DELETE only forgets the skip)
            if mask & (IN_CREATE | IN_ATTRIB | IN_MOVED_TO) != 0 {
                l.rescan = true;
            }
        }
    }
}

/// watch /dev/input and look at its nodes
pub(crate) fn start() -> bool {
    let mut l = LP.lock();
    if l.ino < 0 {
        // SAFETY: no pointers.
        let fd = unsafe { syscall3(SYS_INOTIFY_INIT1, IN_NONBLOCK | IN_CLOEXEC, 0, 0) };
        if fd >= 0 {
            // SAFETY: a NUL-terminated path.
            let wd = unsafe { syscall3(SYS_INOTIFY_ADD_WATCH, fd as usize, c"/dev/input".as_ptr() as usize, (IN_CREATE | IN_ATTRIB | IN_DELETE | IN_MOVED_TO) as usize) };
            if wd < 0 {
                close(fd as usize);
            } else {
                l.ino = fd;
            }
        }
    }
    l.skip.clear();
    l.rescan = true;
    l.next_scan = 0;
    true
}

/// every pad goes, the nodes and the watch close
pub(crate) fn stop() {
    let mut l = LP.lock();
    for d in core::mem::take(&mut l.devs) {
        close(d.fd);
        pad_disconnect(d.slot);
    }
    if l.ino >= 0 {
        close(l.ino as usize);
        l.ino = -1;
    }
    l.skip.clear();
}

/// read what the pads' nodes have; what inotify reported when its time has come (all with full)
/// -> the next time to poll
pub(crate) fn poll(now: u64, focused: bool, full: bool) -> u64 {
    let mut l = LP.lock();
    let refocus = focused != l.focused;
    l.focused = focused;
    if full || now >= l.next_scan {
        if l.ino >= 0 {
            inotify_read(&mut l);
        } else {
            l.rescan = true; // no inotify: look at the folder now and then
        }
        if l.rescan || full {
            l.rescan = false;
            scan(&mut l, focused);
        }
        l.next_scan = now + if l.ino >= 0 { SCAN_US } else { RESCAN_US };
    }
    let mut gone: Vec<usize> = Vec::new();
    let mut buf = [0u8; EVDEV_EVENT_SIZE * 64];
    for (k, d) in l.devs.iter_mut().enumerate() {
        let mut changed = false;
        loop {
            // SAFETY: the buffer is writable for its length.
            let n = unsafe { syscall3(SYS_READ, d.fd, buf.as_mut_ptr() as usize, buf.len()) };
            if n == -EINTR {
                continue;
            }
            if n == -EAGAIN {
                break;
            }
            if n <= 0 {
                gone.push(k); // ENODEV: unplugged (a test's pipe: its writer closed)
                break;
            }
            changed |= d.pad.feed(&buf[..n as usize - n as usize % EVDEV_EVENT_SIZE]);
            if d.pad.needs_resync() {
                let mut held = [0u8; EVDEV_KEYS / 8];
                let mut bits = [0u8; EVDEV_ABS / 8];
                if d.node != u32::MAX && ioctl(d.fd, eviocgkey(held.len()), &mut held) && ioctl(d.fd, eviocgbit(3, bits.len()), &mut bits) {
                    let infos = abs_infos(d.fd, &bits);
                    changed |= d.pad.resync(&held, &infos);
                } else {
                    changed = true;
                }
            }
            if (n as usize) < buf.len() {
                break;
            }
        }
        if changed || refocus {
            pad_update(d.slot, &if focused { d.pad.state() } else { PadState::NEUTRAL });
        }
    }
    for &k in gone.iter().rev() {
        let d = l.devs.remove(k);
        close(d.fd);
        pad_disconnect(d.slot);
    }
    if l.devs.is_empty() { l.next_scan } else { (now + POLL_US).min(l.next_scan) }
}

/// (test aid, plattest --test pad) the writing end of a pipe whose reading end is a pad's node
/// (evdev_test_attach): what is written reaches the pad when the pads are polled; dropping it is
/// the pad's unplugging
pub struct EvdevTestNode(usize);

impl EvdevTestNode {
    /// input_event records (EVDEV_EVENT_SIZE bytes each) into the node -> all written
    pub fn write(&self, data: &[u8]) -> bool {
        // SAFETY: our pipe's writing end; data is readable for its length.
        unsafe { rt::os::syscall3(rt::os::SYS_WRITE, self.0, data.as_ptr() as usize, data.len()) == data.len() as isize }
    }
}

impl Drop for EvdevTestNode {
    fn drop(&mut self) {
        close(self.0);
    }
}

/// (test aid) a pad with these keys and axes (EvdevPad::new) whose node is a pipe of the test's: it
/// connects at once -> (its slot, the pipe's writing end); None: not watching, no slot free or no
/// pipe
pub fn evdev_test_attach(name: &str, vendor: u16, keys: &[u8], abs: &[AbsInfo]) -> Option<(usize, EvdevTestNode)> {
    if !crate::pad_watching() {
        return None;
    }
    let mut fds = [0i32; 2];
    // SAFETY: pipe2 fills two descriptors; O_NONBLOCK | O_CLOEXEC.
    if unsafe { syscall3(rt::os::SYS_PIPE2, fds.as_mut_ptr() as usize, O_NONBLOCK | O_CLOEXEC, 0) } < 0 {
        return None;
    }
    let mut l = LP.lock();
    let Some(slot) = free_slot(&l) else {
        close(fds[0] as usize);
        close(fds[1] as usize);
        return None;
    };
    let pad = EvdevPad::new(vendor, keys, abs);
    let focused = crate::pad_focused();
    pad_connect(slot, name, pad_style(vendor, name), true);
    pad_update(slot, &if focused { pad.state() } else { PadState::NEUTRAL });
    l.devs.push(Dev { fd: fds[0] as usize, node: u32::MAX, slot, pad });
    Some((slot, EvdevTestNode(fds[1] as usize)))
}
