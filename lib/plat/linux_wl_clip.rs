// linux_wl_clip -- clipboard and file drops through the Wayland selection (wl_data_device)
//                                                                   [src/plat/linux/wl_clip.s]
//
//   Wl::clip_set(fmt, data) -> ok      CLIP_TEXT (UTF-8) / CLIP_PNG / CLIP_IMAGE (K1: data =
//        a CLIPIMG record, clipimg_split: offered as image/png + image/bmp).  The data is
//        copied; we offer it until another client takes the selection.
//   Wl::clip_get(fmt) -> bytes         our own selection is returned directly; otherwise the
//        offer is read through a pipe while Wayland events keep being dispatched (3 s
//        timeout, at most CLIP_MAX bytes).  Text preference: text/plain;charset=utf-8,
//        UTF8_STRING, text/plain, STRING / TEXT.  CLIP_PNG: image/png.  CLIP_IMAGE (K1):
//        image/png, else image/bmp (through clip_bmp_fix: paint.net's alpha rule).
//   Wl::clip_has(fmt) -> bool          our selection / the offer has the format (K1)
//   Wl::clip_init()                    the data device (after the globals are bound)
//   Wl::clip_pollfds(pollfds) -> n     pollfds of pending transfers (to other clients, and
//                                      the running drop transfer)
//   Wl::clip_service(pollfds)          continue those transfers (non-blocking)
//   wlh_dd_* / wlh_offer_* / wlh_src_*  event handlers
//
//   Wl::drag_start(data) -> started    (dnd.rs) the app's drag: a wl_data_source with the MIME
//        types of its content, set_actions (copy, move), wl_data_device.start_drag with the
//        serial of the button press (the compositor takes the pointer: its leave releases
//        the buttons); the source's send writes like the clipboard's; cancelled / dnd_finished
//        end it (E_DRAG_END with the negotiated action)
//
// Offers are server-created objects (ids >= 0xFF000000); their udata holds a bit mask of the
// MIME types offered.  Drag and drop (dnd.rs): enter and every motion ask the UI's hook
// (dnd::drag_query with the offer's kinds, the source's actions and the modifier keys) and
// answer with accept(serial, a MIME type of a taken kind / null) and set_actions (v3: the
// taken effects, the preferred one) when the answer changed; E_DRAG goes to the UI (leave:
// data 0).  Drop: the taken kinds are received one after the other through non-blocking pipes
// served by the wait loop (text/uri-list -> paths and URLs, text, text/html, image/png or
// image/bmp; each bounded), then E_DROP (x, y = position in the window's pixels, data = the
// paths) and finish + destroy the offer.

use alloc::vec;
use alloc::vec::Vec;

use rt::os::{syscall3, SYS_READ, SYS_WRITE};

use super::proto::*;
use super::wire::{poll, sys_close, Args, PollFd, A, EAGAIN, EINTR, POLLERR, POLLHUP, POLLIN, POLLOUT};
use super::window::WLWF_USED;
use super::Wl;
use crate::{CLIP_DOCX, CLIP_HTML, CLIP_RTF, CLIP_IMAGE, CLIP_PNG, CLIP_TEXT, DE_ALL, DE_COPY, DE_MOVE, DT_FILES, DT_HTML,
            DT_IMAGE, DT_TEXT, DT_URL, DragData, E_DRAG, E_DROP, PW_MAX};

const MB_UTF8: u64 = 1; // text/plain;charset=utf-8
const MB_UTF8S: u64 = 2; // UTF8_STRING
const MB_TEXT: u64 = 4; // text/plain
const MB_STRING: u64 = 8; // STRING / TEXT
const MB_PNG: u64 = 16; // image/png
const MB_BMP: u64 = 32; // image/bmp
const MB_HTML: u64 = 128;
const MB_RTF: u64 = 256;
const MB_DOCX: u64 = 512;
const MB_HTML_UTF8: u64 = 1024;
const MB_RTF_APP: u64 = 2048;
const MB_URI: u64 = 64; // text/uri-list (K1)
const SRC_ACTS_SHIFT: u32 = 40; // an offer's udata: the source's dnd actions above the MIME bits
const MB_ANYTEXT: u64 = MB_UTF8 | MB_UTF8S | MB_TEXT | MB_STRING;
const WR_MAX: usize = 16; // concurrent transfers to other clients (a clipboard manager takes every format at once)
const READ_TIMEOUT: u64 = 3_000_000; // us
const DND_IDLE_US: u64 = 10_000_000; // a drop source that sends nothing this long is given up
const CLIP_MAX: usize = 1 << 29; // largest clipboard transfer we read (K1)
const DND_MAX: usize = 1 << 20; // largest text/uri-list of a drop (K1)
const DND_IMAGE_MAX: usize = 1 << 28; // largest image of a drop
const DND_ACTION_COPY: u32 = 1;
const DND_ACTION_MOVE: u32 = 2;
const SYS_FCNTL: usize = 72;
const SYS_PIPE2: usize = 293;
const F_SETFL: usize = 4;
const O_NONBLOCK: usize = 0x800;
const O_CLOEXEC: usize = 0x80000;

const MIME_UTF8: &[u8] = b"text/plain;charset=utf-8";
const MIME_UTF8S: &[u8] = b"UTF8_STRING";
const MIME_TEXT: &[u8] = b"text/plain";
const MIME_STRING: &[u8] = b"STRING";
const MIME_PNG: &[u8] = b"image/png";
const MIME_BMP: &[u8] = b"image/bmp";
const MIME_URI: &[u8] = b"text/uri-list";
const MIME_HTML: &[u8] = b"text/html";
const MIME_RTF: &[u8] = b"text/rtf";
const MIME_DOCX: &[u8] = b"application/x-jfgui-docx-fragment";
const MIME_HTML_UTF8: &[u8] = b"text/html;charset=utf-8";
const MIME_RTF_APP: &[u8] = b"application/rtf";
const RICH_MIMES: [(u32, &[u8], u64); 5] = [
    (CLIP_HTML, MIME_HTML, MB_HTML), (CLIP_RTF, MIME_RTF, MB_RTF), (CLIP_DOCX, MIME_DOCX, MB_DOCX),
    (CLIP_HTML, MIME_HTML_UTF8, MB_HTML_UTF8), (CLIP_RTF, MIME_RTF_APP, MB_RTF_APP),
];
// offered for text, in this order
const TEXT_MIMES: [&[u8]; 5] = [MIME_UTF8, MIME_UTF8S, MIME_TEXT, MIME_STRING, b"TEXT"];

static MIME_TABLE: [(&[u8], u64); 13] = [(MIME_UTF8, MB_UTF8), (MIME_UTF8S, MB_UTF8S), (MIME_TEXT, MB_TEXT),
                                       (MIME_STRING, MB_STRING), (b"TEXT", MB_STRING), (MIME_PNG, MB_PNG),
                                       (MIME_BMP, MB_BMP), (MIME_URI, MB_URI), (MIME_HTML, MB_HTML), (MIME_RTF, MB_RTF), (MIME_DOCX, MB_DOCX), (MIME_HTML_UTF8, MB_HTML_UTF8), (MIME_RTF_APP, MB_RTF_APP)];

/// a transfer to another client (fd 0 = free slot; fds are never 0 here)
#[derive(Clone, Copy, Default)]
struct Writer {
    fd: i32,
    off: usize,
    bmp: bool, // sends the image/bmp copy, else the data
    rich: u32, // clipboard_platform: rich format, 0 = primary text/image
    dnd: usize, // the app's drag: entry + 1 of ds_data (0 = the clipboard)
}

#[derive(Default)]
pub(crate) struct Clip {
    src_data: Option<Vec<u8>>,
    src_bmp: Option<Vec<u8>>, // CLIP_IMAGE: the image/bmp bytes
    src_fmt: u32,
    src_rich: Vec<(u32, Vec<u8>)>, // clipboard_platform: alternatives of the same selection
    dnd_buf: Option<Vec<u8>>, // drop transfer: the bytes of the MIME type being read
    writers: [Writer; WR_MAX],
    ddev: u32,
    src: u32,
    sel_offer: u32,
    dnd_offer: u32, // offer of the drag over our window
    dnd_action: u32,
    dnd_serial: u32, // of its enter (accept)
    dnd_srcacts: u32, // the source's actions (v3; 0 = unknown)
    dnd_types: u32, // the DT_* kinds it offers
    dnd_answer: (u32, u32), // what we answered last (kinds, effect)
    dnd_shown: Option<(i32, i32)>, // the last E_DRAG point
    dnd_win: i32,
    dnd_x: i32,
    dnd_y: i32,
    dnd_xoffer: u32, // offer of the running drop transfer
    dnd_xaction: u32,
    dnd_xwin: i32,
    dnd_xx: i32,
    dnd_xy: i32,
    dnd_xeffect: u32,
    dnd_xfiles: u32,
    dnd_queue: Vec<(u32, &'static [u8])>, // the kinds still to read: (DT_* kind, MIME type)
    dnd_kind: (u32, &'static [u8]), // the one being read
    dnd_rfd: i32, // its pipe (0 = none)
    dnd_got: bool, // some data of the drop was taken
    dnd_deadline: u64, // time_us when the drop transfer gives up (no data for DND_IDLE_US)
    // the app's drag (drag_start)
    ds: u32, // its wl_data_source (0 = none)
    ds_data: Vec<(&'static [u8], Vec<u8>)>,
    ds_action: u32,
    ds_win: u8,
}

/// MB_* bit of a MIME type name (0 = not used)  [mime_bit]
fn mime_bit(s: &[u8]) -> u64 {
    MIME_TABLE.iter().find(|m| m.0 == s).map_or(0, |m| m.1)
}

fn pipe2(flags: usize) -> Option<[i32; 2]> {
    let mut p = [0i32; 2];
    // SAFETY: pipe2 writes two descriptors.
    let r = unsafe { syscall3(SYS_PIPE2, p.as_mut_ptr() as usize, flags, 0) };
    if r == 0 { Some(p) } else { None }
}

impl Wl {
    /// wl_data_device_manager.get_data_device(seat)  [wl_clip_init]
    pub(crate) fn clip_init(&mut self) {
        if self.g.ddm == 0 || self.g.seat == 0 {
            return;
        }
        self.clip.ddev =
            self.w.send(self.g.ddm, &DDM_GET_DATA_DEVICE, &[A::N(&IFACE_WL_DATA_DEVICE), A::O(self.g.seat)]);
    }

    /// pixels of a drag position (the window's scale)  [dnd_px]
    fn dnd_px(&self, fixed: i32, w: i32) -> i32 {
        let r = &self.win[w as usize & (PW_MAX - 1)];
        let s = if r.flags & WLWF_USED != 0 { r.s120 } else { self.inp.s120_test };
        (s as i64 * fixed as i64 / 30720) as i32
    }

    /// destroy the current drag-and-drop offer  [dnd_drop_offer]
    fn dnd_drop_offer(&mut self) {
        let o = self.clip.dnd_offer;
        self.clip.dnd_offer = 0;
        if o != 0 && o != self.clip.sel_offer {
            self.w.send(o, &DATA_OFFER_DESTROY, &[]);
        }
    }

    /// the drop transfer is over: finish (when an action was negotiated) and destroy the
    /// offer, close the pipe, free the buffer  [dnd_end]
    fn dnd_end(&mut self, ok: bool) {
        let fd = self.clip.dnd_rfd;
        self.clip.dnd_rfd = 0;
        if fd != 0 {
            sys_close(fd);
        }
        let o = self.clip.dnd_xoffer;
        self.clip.dnd_xoffer = 0;
        if o != 0 {
            if ok && self.g.ddm_ver >= 3 && self.clip.dnd_xaction != 0 {
                self.w.send(o, &DATA_OFFER_FINISH, &[]);
            }
            if o != self.clip.sel_offer {
                self.w.send(o, &DATA_OFFER_DESTROY, &[]);
                self.w.flush();
            }
        }
        self.clip.dnd_buf = None;
        self.clip.dnd_queue.clear();
    }

    /// the DT_* kinds of an offer's MIME mask
    fn dnd_kinds(mask: u64) -> u32 {
        let mut t = 0;
        if mask & MB_URI != 0 {
            t |= DT_FILES | DT_URL;
        }
        if mask & MB_ANYTEXT != 0 {
            t |= DT_TEXT;
        }
        if mask & (MB_HTML | MB_HTML_UTF8) != 0 {
            t |= DT_HTML;
        }
        if mask & (MB_PNG | MB_BMP) != 0 {
            t |= DT_IMAGE;
        }
        t
    }

    /// the MIME types to receive for the taken kinds of an offer's mask, in order
    fn dnd_mimes(types: u32, mask: u64) -> Vec<(u32, &'static [u8])> {
        let mut v: Vec<(u32, &'static [u8])> = Vec::new();
        if types & (DT_FILES | DT_URL) != 0 && mask & MB_URI != 0 {
            v.push((types & (DT_FILES | DT_URL), MIME_URI));
        }
        if types & DT_TEXT != 0 {
            if let Some(m) = [(MB_UTF8, MIME_UTF8), (MB_UTF8S, MIME_UTF8S), (MB_TEXT, MIME_TEXT), (MB_STRING, MIME_STRING)]
                .iter()
                .find(|m| mask & m.0 != 0)
            {
                v.push((DT_TEXT, m.1));
            }
        }
        if types & DT_HTML != 0 {
            if mask & MB_HTML != 0 {
                v.push((DT_HTML, MIME_HTML));
            } else if mask & MB_HTML_UTF8 != 0 {
                v.push((DT_HTML, MIME_HTML_UTF8));
            }
        }
        if types & DT_IMAGE != 0 {
            if mask & MB_PNG != 0 {
                v.push((DT_IMAGE, MIME_PNG));
            } else if mask & MB_BMP != 0 {
                v.push((DT_IMAGE, MIME_BMP));
            }
        }
        v
    }

    /// enter / motion: ask the UI, answer the offer when the answer changed, E_DRAG when the
    /// point changed
    fn dnd_answer(&mut self) {
        let o = self.clip.dnd_offer;
        if o == 0 {
            return;
        }
        let Some((_, mask, _)) = self.w.obj_iface(o) else { return };
        let c = &self.clip;
        let (w, x, y, offer) = (c.dnd_win, c.dnd_x, c.dnd_y, c.dnd_types);
        let allowed = if c.dnd_srcacts == 0 { DE_COPY | DE_MOVE } else { c.dnd_srcacts & (DE_COPY | DE_MOVE) };
        let (t, e) = crate::drag_query(w, x, y, offer, allowed, super::xkb::wl_mods() as u8);
        if (t, e) != self.clip.dnd_answer {
            self.clip.dnd_answer = (t, e);
            let mime = Self::dnd_mimes(t, mask).first().map(|m| m.1); // accept(serial, NULL): nothing for us
            let serial = self.clip.dnd_serial;
            self.w.send(o, &DATA_OFFER_ACCEPT, &[A::U(serial), A::S(mime)]);
            if self.g.ddm_ver >= 3 {
                // the effects we take (no move for files: drag_query), the preferred one
                let acts = if t == 0 { 0 } else if t & DT_FILES != 0 { DND_ACTION_COPY } else { allowed & (DND_ACTION_COPY | DND_ACTION_MOVE) };
                self.w.send(o, &DATA_OFFER_SET_ACTIONS, &[A::U(acts), A::U(e & acts)]);
            }
            self.w.flush();
        }
        if self.clip.dnd_shown != Some((x, y)) && offer != 0 {
            self.clip.dnd_shown = Some((x, y));
            self.inp.evwin = w;
            self.push(E_DRAG, 0, x, y, offer | allowed << 8);
        }
    }

    /// receive the next queued MIME type of the drop -> false when there is none (or no pipe)
    fn dnd_receive_next(&mut self) -> bool {
        let o = self.clip.dnd_xoffer;
        if o == 0 || self.clip.dnd_queue.is_empty() {
            return false;
        }
        let k = self.clip.dnd_queue.remove(0);
        let Some(p) = pipe2(O_CLOEXEC | O_NONBLOCK) else { return false };
        self.clip.dnd_kind = k;
        self.clip.dnd_buf = Some(Vec::with_capacity(4096));
        // write end: sent, then closed by flush
        self.w.send(o, &DATA_OFFER_RECEIVE, &[A::S(Some(k.1)), A::H(p[1])]);
        self.w.flush();
        self.clip.dnd_rfd = p[0];
        true
    }

    /// one MIME type of the drop arrived: into the pending drop
    fn dnd_take(&mut self, kind: u32, mime: &[u8], buf: &[u8]) {
        if mime == MIME_URI {
            if kind & DT_FILES != 0 {
                self.clip.dnd_xfiles += crate::drop_uri_list(buf);
            }
            if kind & DT_URL != 0 {
                crate::drop_url_list(buf);
            }
        } else if kind == DT_TEXT {
            crate::drop_set_text(buf);
        } else if kind == DT_HTML {
            let h = if buf.len() >= 2 && buf[0] == 0xFF && buf[1] == 0xFE { crate::drop_text_from(&buf[2..], true) } else { buf.to_vec() };
            crate::drop_set_html(&h);
        } else if kind == DT_IMAGE {
            let img = if mime == MIME_BMP { crate::clip_bmp_fix(buf) } else { Some(buf.to_vec()) };
            if let Some(i) = img {
                crate::drop_set_image(i);
            }
        }
    }

    // the drop's reads are done: E_DROP, finish -- or, when nothing could be taken (too
    // large, empty), no drop: not finished, so a moving source keeps its data
    fn dnd_done(&mut self) {
        if !self.clip.dnd_got {
            self.inp.evwin = self.clip.dnd_xwin;
            self.push(E_DRAG, 0, -1, -1, 0);
            self.dnd_end(false);
            return;
        }
        crate::drop_set_effect(self.clip.dnd_xeffect);
        self.inp.evwin = self.clip.dnd_xwin;
        let (x, y, n) = (self.clip.dnd_xx, self.clip.dnd_xy, self.clip.dnd_xfiles);
        self.push(E_DROP, 0, x, y, n);
        self.dnd_end(true);
    }

    /// the drop pipe is readable: read what is there; at its end the next MIME type, after the
    /// last one E_DROP  [dnd_read]
    fn dnd_read(&mut self) {
        loop {
            let limit = match self.clip.dnd_kind.0 {
                DT_IMAGE => DND_IMAGE_MAX,
                DT_TEXT | DT_HTML => crate::CLIP_RICH_MAX,
                _ => DND_MAX,
            };
            let Some(buf) = self.clip.dnd_buf.as_mut() else { return };
            if buf.capacity() - buf.len() <= 1024 {
                let cap = buf.capacity() * 2;
                if cap > limit.next_power_of_two() * 2 {
                    self.dnd_end(false); // too large
                    return;
                }
                buf.reserve_exact(cap - buf.len());
            }
            let len = buf.len();
            let room = buf.capacity() - len;
            // SAFETY: reads into the spare capacity of the buffer.
            let r = unsafe { syscall3(SYS_READ, self.clip.dnd_rfd as usize, buf.as_mut_ptr().add(len) as usize, room) };
            if r == -EINTR {
                continue;
            }
            if r == -EAGAIN {
                return; // more later
            }
            if r < 0 {
                self.dnd_end(false);
                return;
            }
            if r == 0 {
                let data = self.clip.dnd_buf.take().unwrap_or_default();
                let (kind, mime) = self.clip.dnd_kind;
                sys_close(self.clip.dnd_rfd);
                self.clip.dnd_rfd = 0;
                if data.len() <= limit && !data.is_empty() {
                    self.dnd_take(kind, mime, &data);
                    self.clip.dnd_got = true;
                }
                if !self.dnd_receive_next() {
                    self.dnd_done();
                }
                return;
            }
            // SAFETY: the kernel wrote r bytes behind len.
            unsafe { buf.set_len(len + r as usize) };
            self.clip.dnd_deadline = crate::time_us() + DND_IDLE_US;
        }
    }

    /// (wait) a drop transfer whose source sends nothing gives up -> the next deadline
    pub(crate) fn clip_tick(&mut self, now: u64) -> u64 {
        if self.clip.dnd_rfd == 0 || self.clip.dnd_deadline == 0 {
            return u64::MAX;
        }
        if now < self.clip.dnd_deadline {
            return self.clip.dnd_deadline;
        }
        self.clip.dnd_deadline = 0;
        self.inp.evwin = self.clip.dnd_xwin;
        self.push(E_DRAG, 0, -1, -1, 0);
        self.dnd_end(false); // (not finished: a moving source keeps its data)
        u64::MAX
    }

    // ------------------------------------------------------------------ source (copy side)

    /// stop offering our data: destroy the source, end transfers  [clip_drop_source]
    fn clip_drop_source(&mut self) {
        let s = self.clip.src;
        if s != 0 {
            self.clip.src = 0;
            self.w.send(s, &DATA_SOURCE_DESTROY, &[]);
        }
        for wr in self.clip.writers.iter_mut() {
            if wr.fd != 0 && wr.dnd == 0 {
                sys_close(wr.fd); // (the transfers of the app's drag go on)
                wr.fd = 0;
            }
        }
        self.clip.src_data = None;
        self.clip.src_bmp = None;
        self.clip.src_rich.clear();
    }

    /// offer data on the clipboard -> ok  [wl_plat_clip_set]
    pub(crate) fn clip_set(&mut self, fmt: u32, data: &[u8]) -> bool {
        if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) { return self.clip_set_many(&[(fmt, data)]); }
        if self.clip.ddev == 0 {
            return false;
        }
        let (payload, bmp) = match fmt {
            CLIP_TEXT | CLIP_PNG => (data, None),
            CLIP_IMAGE => {
                let Some((png, bmp)) = crate::clipimg_split(data) else { return false };
                if png.is_empty() {
                    return false;
                }
                (png, if bmp.is_empty() { None } else { Some(bmp.to_vec()) })
            }
            _ => return false,
        };
        let copy = payload.to_vec();
        self.clip_drop_source();
        let has_bmp = bmp.is_some();
        self.clip.src_data = Some(copy);
        self.clip.src_bmp = bmp;
        self.clip.src_fmt = fmt;
        let src = self.w.send(self.g.ddm, &DDM_CREATE_DATA_SOURCE, &[A::N(&IFACE_WL_DATA_SOURCE)]);
        self.clip.src = src;
        if fmt == CLIP_TEXT {
            for m in TEXT_MIMES {
                self.w.send(src, &DATA_SOURCE_OFFER, &[A::S(Some(m))]);
            }
        } else {
            self.w.send(src, &DATA_SOURCE_OFFER, &[A::S(Some(MIME_PNG))]);
            if has_bmp {
                self.w.send(src, &DATA_SOURCE_OFFER, &[A::S(Some(MIME_BMP))]);
            }
        }
        let (dd, serial) = (self.clip.ddev, self.inp.input_serial);
        self.w.send(dd, &DATA_DEVICE_SET_SELECTION, &[A::O(src), A::U(serial)]);
        self.w.flush();
        true
    }

    // clipboard_platform: offer every representation under a single wl_data_source.
    pub(crate) fn clip_set_many(&mut self, entries: &[(u32, &[u8])]) -> bool {
        if self.clip.ddev == 0 || !crate::clip_many_valid(entries) { return false; }
        let text = entries.iter().find(|e| e.0 == CLIP_TEXT).map(|e| e.1.to_vec());
        let rich = entries.iter().filter(|e| e.0 != CLIP_TEXT).map(|e| (e.0, e.1.to_vec())).collect();
        self.clip_drop_source();
        self.clip.src_fmt = if text.is_some() { CLIP_TEXT } else { 0 };
        self.clip.src_data = text;
        self.clip.src_rich = rich;
        let src = self.w.send(self.g.ddm, &DDM_CREATE_DATA_SOURCE, &[A::N(&IFACE_WL_DATA_SOURCE)]);
        self.clip.src = src;
        for &(fmt, _) in entries {
            if fmt == CLIP_TEXT {
                for mime in TEXT_MIMES { self.w.send(src, &DATA_SOURCE_OFFER, &[A::S(Some(mime))]); }
            } else {
                for (_, mime, _) in RICH_MIMES.iter().filter(|m| m.0 == fmt) {
                    self.w.send(src, &DATA_SOURCE_OFFER, &[A::S(Some(mime))]);
                }
            }
        }
        self.w.send(self.clip.ddev, &DATA_DEVICE_SET_SELECTION, &[A::O(src), A::U(self.inp.input_serial)]);
        self.w.flush();
        true
    }

    /// write as much as possible; close when done or failed  [writer_run]
    fn writer_run(&mut self, k: usize) {
        loop {
            let wr = self.clip.writers[k];
            let data: &[u8] = if wr.dnd != 0 {
                self.clip.ds_data.get(wr.dnd - 1).map_or(&[], |e| e.1.as_slice()) // the app's drag
            } else if wr.rich != 0 {
                self.clip.src_rich.iter().find(|e| e.0 == wr.rich).map_or(&[], |e| e.1.as_slice())
            } else if wr.bmp {
                self.clip.src_bmp.as_deref().unwrap_or(&[])
            } else {
                self.clip.src_data.as_deref().unwrap_or(&[])
            };
            if wr.off >= data.len() {
                break;
            }
            let rest = &data[wr.off..];
            // SAFETY: writes from our data.
            let r = unsafe { syscall3(SYS_WRITE, wr.fd as usize, rest.as_ptr() as usize, rest.len()) };
            if r == -EINTR {
                continue;
            }
            if r == -EAGAIN {
                return; // continue when poll says POLLOUT
            }
            if r <= 0 {
                break; // EPIPE etc.
            }
            self.clip.writers[k].off += r as usize;
        }
        sys_close(self.clip.writers[k].fd);
        self.clip.writers[k].fd = 0;
    }

    /// pollfds of the transfers (POLLOUT for every writer, POLLIN for the drop transfer)
    /// -> entries written  [wl_clip_pollfds]
    pub(crate) fn clip_pollfds(&self, fds: &mut [PollFd]) -> usize {
        let mut n = 0;
        for wr in &self.clip.writers {
            if n >= fds.len() {
                return n;
            }
            if wr.fd != 0 {
                fds[n] = PollFd { fd: wr.fd, events: POLLOUT, revents: 0 };
                n += 1;
            }
        }
        if n < fds.len() && self.clip.dnd_rfd != 0 {
            fds[n] = PollFd { fd: self.clip.dnd_rfd, events: POLLIN, revents: 0 };
            n += 1;
        }
        n
    }

    /// continue transfers whose fd became ready  [wl_clip_service]
    pub(crate) fn clip_service(&mut self, fds: &[PollFd]) {
        for p in fds {
            if p.revents == 0 {
                continue;
            }
            if p.fd == self.clip.dnd_rfd && p.fd != 0 {
                self.dnd_read();
                continue;
            }
            if let Some(k) = self.clip.writers.iter().position(|w| w.fd == p.fd) {
                self.writer_run(k);
            }
        }
    }

    // ------------------------------------------------------------------ paste

    /// our own selection serves fmt  [own_has]
    fn own_has(&self, fmt: u32) -> bool {
        if self.clip.src_rich.iter().any(|e| e.0 == fmt) { return true; }
        let f = self.clip.src_fmt;
        if fmt == CLIP_TEXT {
            return f == CLIP_TEXT;
        }
        (f == CLIP_PNG || f == CLIP_IMAGE) && (fmt == CLIP_PNG || fmt == CLIP_IMAGE)
    }

    /// the clipboard offers fmt (K1)  [wl_plat_clip_has]
    pub(crate) fn clip_has(&self, fmt: u32) -> bool {
        if self.clip.src != 0 {
            return self.own_has(fmt);
        }
        let o = self.clip.sel_offer;
        if o == 0 {
            return false;
        }
        let Some((_, mask, _)) = self.w.obj_iface(o) else { return false };
        let want = match fmt {
            CLIP_TEXT => MB_ANYTEXT,
            CLIP_PNG => MB_PNG,
            CLIP_IMAGE => MB_PNG | MB_BMP,
            CLIP_HTML => MB_HTML | MB_HTML_UTF8,
            CLIP_RTF => MB_RTF | MB_RTF_APP,
            CLIP_DOCX => MB_DOCX,
            _ => 0,
        };
        mask & want != 0
    }

    /// read the clipboard  [wl_plat_clip_get]
    pub(crate) fn clip_get(&mut self, fmt: u32) -> Option<Vec<u8>> {
        // our own selection
        if self.clip.src != 0 {
            if !self.own_has(fmt) {
                return None;
            }
            return if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) {
                self.clip.src_rich.iter().find(|e| e.0 == fmt).map(|e| e.1.clone())
            } else { self.clip.src_data.clone() };
        }
        let o = self.clip.sel_offer;
        if o == 0 {
            return None;
        }
        let (_, mask, _) = self.w.obj_iface(o)?;
        let mime: Option<&[u8]> = match fmt {
            CLIP_HTML | CLIP_RTF | CLIP_DOCX => RICH_MIMES.iter().find(|m| m.0 == fmt && mask & m.2 != 0).map(|m| m.1),
            CLIP_PNG => (mask & MB_PNG != 0).then_some(MIME_PNG),
            CLIP_IMAGE => {
                if mask & MB_PNG != 0 {
                    Some(MIME_PNG)
                } else if mask & MB_BMP != 0 {
                    Some(MIME_BMP)
                } else {
                    None
                }
            }
            CLIP_TEXT => [(MB_UTF8, MIME_UTF8), (MB_UTF8S, MIME_UTF8S), (MB_TEXT, MIME_TEXT), (MB_STRING, MIME_STRING)]
                .iter()
                .find(|m| mask & m.0 != 0)
                .map(|m| m.1),
            _ => return None,
        };
        let Some(mime) = mime else {
            if self.w.debug {
                crate::write_stderr(alloc::format!("[wl] clipboard: no usable format (offer mask {mask:x})\n").as_bytes());
            }
            return None;
        };
        let p = pipe2(O_CLOEXEC)?;
        // write end: sent, then closed by flush
        self.w.send(o, &DATA_OFFER_RECEIVE, &[A::S(Some(mime)), A::H(p[1])]);
        self.w.flush_wait();
        let mut buf: Vec<u8> = vec![0; 65536];
        let mut len = 0usize;
        let rich = matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX);
        // 3 s without data (not 3 s in all: large data, made while it is read, takes longer)
        let mut deadline = crate::time_us() + READ_TIMEOUT;
        loop {
            let now = crate::time_us();
            if now >= deadline {
                if self.w.debug {
                    crate::write_stderr(b"[wl] clipboard: no data for 3 s\n");
                }
                sys_close(p[0]);
                return None;
            }
            let ms = ((deadline - now) / 1000) as i32 + 1;
            let mut fds = [PollFd { fd: p[0], events: POLLIN, revents: 0 }, PollFd { fd: self.w.fd, events: POLLIN,
                                                                                    revents: 0 }];
            let r = poll(&mut fds, ms);
            if r == -EINTR {
                continue;
            }
            if r < 0 {
                sys_close(p[0]);
                return None;
            }
            if fds[1].revents & (POLLIN | POLLERR | POLLHUP) != 0 {
                self.w.recv(); // keep the connection alive (pings, ...)
                self.dispatch();
            }
            if fds[0].revents & (POLLIN | POLLERR | POLLHUP) == 0 {
                continue;
            }
            if buf.len() - len <= 4096 {
                let cap = if rich { (buf.len() * 2).min(crate::CLIP_RICH_MAX + 1) } else { buf.len() * 2 };
                if !rich && cap > CLIP_MAX {
                    sys_close(p[0]); // K1: other programs' data is bounded
                    return None;
                }
                buf.resize(cap, 0);
            }
            let room = buf.len() - len - usize::from(!rich);
            // SAFETY: reads into buf[len..len + room].
            let r = unsafe { syscall3(SYS_READ, p[0] as usize, buf.as_mut_ptr().add(len) as usize, room) };
            if r == -EINTR {
                continue;
            }
            if r < 0 {
                sys_close(p[0]);
                return None;
            }
            if r == 0 {
                break;
            }
            len += r as usize;
            deadline = crate::time_us() + READ_TIMEOUT;
            if rich && len > crate::CLIP_RICH_MAX {
                sys_close(p[0]);
                return None;
            }
        }
        sys_close(p[0]);
        buf.truncate(len);
        if mime == MIME_BMP {
            return crate::clip_bmp_fix(&buf); // paint.net's alpha rule (a new buffer)
        }
        Some(buf)
    }
}

// ------------------------------------------------------------------ offers (paste side)

/// wl_data_device.data_offer(new id): track the new offer  [wlh_dd_offer]
pub(crate) fn wlh_dd_offer(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.w.srv_add(a.u(0), &IFACE_WL_DATA_OFFER, 0);
}

/// wl_data_offer.offer(mime)  [wlh_offer_mime]
pub(crate) fn wlh_offer_mime(wl: &mut Wl, id: u32, a: &Args, u: u64) {
    if let Some(m) = a.s(0) {
        wl.w.set_udata(id, u | mime_bit(m));
    }
}

/// wl_data_offer.source_actions(actions) (v3): what the source of the drag allows
/// [wlh_offer_src_actions]
pub(crate) fn wlh_offer_src_actions(wl: &mut Wl, id: u32, a: &Args, u: u64) {
    // kept in the offer (bits 40..): it comes before the enter of its drag
    wl.w.set_udata(id, (u & !(7 << SRC_ACTS_SHIFT)) | ((a.u(0) & 7) as u64) << SRC_ACTS_SHIFT);
    if id == wl.clip.dnd_offer {
        wl.clip.dnd_srcacts = a.u(0);
        wl.dnd_answer();
    }
}

/// wl_data_offer.action(dnd_action) (K1): the negotiated action of the drag-and-drop offer
/// [wlh_offer_action]
pub(crate) fn wlh_offer_action(wl: &mut Wl, id: u32, a: &Args, _u: u64) {
    if id == wl.clip.dnd_offer {
        wl.clip.dnd_action = a.u(0);
    } else if id == wl.clip.dnd_xoffer {
        wl.clip.dnd_xaction = a.u(0);
    }
}

/// the clipboard now holds this offer (or nothing)  [wlh_dd_selection]
pub(crate) fn wlh_dd_selection(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let new = a.u(0);
    let old = wl.clip.sel_offer;
    if old != 0 && old != new && old != wl.clip.dnd_offer && old != wl.clip.dnd_xoffer {
        wl.w.send(old, &DATA_OFFER_DESTROY, &[]);
    }
    wl.clip.sel_offer = new;
}

/// drag-and-drop enter(serial, surface, x, y, offer): the kinds it offers; the UI's answer
/// (accept, set_actions) and E_DRAG  [wlh_dd_enter]
pub(crate) fn wlh_dd_enter(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.dnd_drop_offer();
    let offer = a.u(4);
    let w = wl.find(a.u(1)).max(0);
    let c = &mut wl.clip;
    c.dnd_offer = offer;
    c.dnd_action = 0;
    c.dnd_serial = a.u(0);
    c.dnd_answer = (u32::MAX, 0); // the first answer always goes out
    c.dnd_shown = None;
    c.dnd_win = w;
    c.dnd_types = 0;
    wl.clip.dnd_x = wl.dnd_px(a.i(2), w);
    wl.clip.dnd_y = wl.dnd_px(a.i(3), w);
    if offer == 0 {
        return;
    }
    let Some((_, mask, _)) = wl.w.obj_iface(offer) else { return };
    wl.clip.dnd_types = Wl::dnd_kinds(mask);
    wl.clip.dnd_srcacts = ((mask >> SRC_ACTS_SHIFT) & 7) as u32;
    wl.dnd_answer();
}

/// motion(time, x, y) during a drag  [wlh_dd_motion]
pub(crate) fn wlh_dd_motion(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let w = wl.clip.dnd_win;
    wl.clip.dnd_x = wl.dnd_px(a.i(1), w);
    wl.clip.dnd_y = wl.dnd_px(a.i(2), w);
    wl.dnd_answer();
}

/// drag-and-drop leave (also after a drop): E_DRAG with data 0  [wlh_dd_leave]
pub(crate) fn wlh_dd_leave(wl: &mut Wl, _id: u32, _a: &Args, _u: u64) {
    if wl.clip.dnd_offer != 0 {
        wl.inp.evwin = wl.clip.dnd_win;
        wl.push(E_DRAG, 0, -1, -1, 0);
    }
    wl.dnd_drop_offer();
}

/// drop: receive the taken kinds one after the other through pipes  [wlh_dd_drop]
pub(crate) fn wlh_dd_drop(wl: &mut Wl, _id: u32, _a: &Args, _u: u64) {
    let o = wl.clip.dnd_offer;
    if o == 0 {
        return;
    }
    let (t, e) = wl.clip.dnd_answer;
    let mask = wl.w.obj_iface(o).map_or(0, |x| x.1);
    let queue = Wl::dnd_mimes(if t == u32::MAX { 0 } else { t }, mask);
    if queue.is_empty() || e == 0 || wl.clip.dnd_rfd != 0 {
        wl.inp.evwin = wl.clip.dnd_win;
        wl.push(E_DRAG, 0, -1, -1, 0);
        wl.dnd_drop_offer(); // nothing taken, or one transfer at a time
        return;
    }
    crate::drop_reset();
    let c = &mut wl.clip;
    c.dnd_xoffer = o;
    c.dnd_xaction = c.dnd_action;
    c.dnd_xwin = c.dnd_win;
    c.dnd_xx = c.dnd_x;
    c.dnd_xy = c.dnd_y;
    c.dnd_xfiles = 0;
    // the negotiated action wins over our preference (v3)
    c.dnd_xeffect = match c.dnd_action {
        DND_ACTION_MOVE if e & DE_ALL != 0 => DE_MOVE,
        DND_ACTION_COPY => DE_COPY,
        _ => e,
    };
    c.dnd_queue = queue;
    c.dnd_got = false;
    c.dnd_deadline = crate::time_us() + DND_IDLE_US;
    c.dnd_offer = 0; // leave must not destroy it
    if !wl.dnd_receive_next() {
        wl.dnd_end(false);
    }
}

// ------------------------------------------------------------------ source events

/// send(mime, fd): write our data to fd (non-blocking)  [wlh_src_send]
pub(crate) fn wlh_src_send(wl: &mut Wl, id: u32, a: &Args, _u: u64) {
    let fd = a.u(1) as i32;
    let dnd = if id != 0 && id == wl.clip.ds {
        match wl.clip.ds_data.iter().position(|e| a.s(0) == Some(e.0)) {
            Some(k) => k + 1, // the app's drag
            None => {
                sys_close(fd);
                return;
            }
        }
    } else {
        0
    };
    if dnd == 0 && (id != wl.clip.src || (wl.clip.src_data.is_none() && wl.clip.src_rich.is_empty())) {
        sys_close(fd);
        return;
    }
    let Some(k) = wl.clip.writers.iter().position(|w| w.fd == 0) else {
        sys_close(fd); // too many readers at once
        return;
    };
    // O_NONBLOCK so we never stall the UI
    // SAFETY: fcntl on the fd the compositor gave us.
    unsafe { syscall3(SYS_FCNTL, fd as usize, F_SETFL, O_NONBLOCK) };
    let bmp = dnd == 0 && wl.clip.src_bmp.is_some() && a.s(0) == Some(MIME_BMP); // image/bmp -> the BMP copy
    let rich = if dnd != 0 { 0 } else { RICH_MIMES.iter().find(|m| a.s(0) == Some(m.1)).map_or(0, |m| m.0) };
    if rich != 0 && !wl.clip.src_rich.iter().any(|e| e.0 == rich) { sys_close(fd); return; }
    wl.clip.writers[k] = Writer { fd, off: 0, bmp, rich, dnd };
    wl.writer_run(k);
}

/// another client owns the selection now / the app's drag was cancelled  [wlh_src_cancelled]
pub(crate) fn wlh_src_cancelled(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    if id == wl.clip.ds && id != 0 {
        wl.drag_end(0); // refused or cancelled
    } else if id == wl.clip.src {
        wl.clip_drop_source();
    } else {
        wl.w.send(id, &DATA_SOURCE_DESTROY, &[]);
    }
}

/// dnd_finished (v3): the target took the app's drag with the negotiated action
/// [wlh_src_dnd_finished]
pub(crate) fn wlh_src_dnd_finished(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    if id == wl.clip.ds && id != 0 {
        let e = if wl.clip.ds_action == DND_ACTION_MOVE { DE_MOVE } else { DE_COPY };
        wl.drag_end(e);
    }
}

/// action(dnd_action) (v3): the action the compositor negotiated for the app's drag
/// [wlh_src_action]
pub(crate) fn wlh_src_action(wl: &mut Wl, id: u32, a: &Args, _u: u64) {
    if id == wl.clip.ds && id != 0 {
        wl.clip.ds_action = a.u(0);
    }
}

/// dnd_drop_performed (v3): dropped; for a data device below v3 this is the end
/// [wlh_src_dropped]
pub(crate) fn wlh_src_dropped(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    if id == wl.clip.ds && id != 0 && wl.g.ddm_ver < 3 {
        wl.drag_end(DE_COPY);
    }
}

impl Wl {
    /// (dnd.rs) drag `d` out of the app while the left button is held -> started
    pub(crate) fn drag_start(&mut self, d: DragData) -> bool {
        let w = self.inp.ptr_win;
        if self.clip.ddev == 0 || self.clip.ds != 0 || !self.left_held() || w < 0 {
            return false;
        }
        let surf = self.win[w as usize & (PW_MAX - 1)].surf;
        if surf == 0 {
            return false;
        }
        let mut data: Vec<(&'static [u8], Vec<u8>)> = Vec::new();
        if !d.files.is_empty() || !d.urls.is_empty() {
            let mut l = Vec::new();
            for f in &d.files {
                l.extend_from_slice(&uri_of(&abs_path(f)));
                l.extend_from_slice(b"\r\n");
            }
            for u in &d.urls {
                l.extend_from_slice(u.as_bytes());
                l.extend_from_slice(b"\r\n");
            }
            data.push((MIME_URI, l));
        }
        let text = d.text.clone().or_else(|| if d.urls.is_empty() { None } else { Some(d.urls.join("\n")) });
        if let Some(t) = text {
            for m in TEXT_MIMES {
                data.push((m, t.clone().into_bytes()));
            }
        }
        if let Some(h) = &d.html {
            data.push((MIME_HTML, h.clone().into_bytes()));
        }
        if let Some(p) = &d.image {
            data.push((MIME_PNG, p.clone()));
        }
        if let Some(b) = &d.bmp {
            data.push((MIME_BMP, b.clone()));
        }
        if data.is_empty() {
            return false;
        }
        let src = self.w.send(self.g.ddm, &DDM_CREATE_DATA_SOURCE, &[A::N(&IFACE_WL_DATA_SOURCE)]);
        for (m, _) in &data {
            self.w.send(src, &DATA_SOURCE_OFFER, &[A::S(Some(m))]);
        }
        if self.g.ddm_ver >= 3 {
            let a = d.allowed();
            let acts = (if a & DE_COPY != 0 { DND_ACTION_COPY } else { 0 }) | (if a & DE_MOVE != 0 { DND_ACTION_MOVE } else { 0 });
            self.w.send(src, &DATA_SOURCE_SET_ACTIONS, &[A::U(acts.max(DND_ACTION_COPY))]);
        }
        let (dd, serial) = (self.clip.ddev, self.inp.btn_serial);
        self.w.send(dd, &DATA_DEVICE_START_DRAG, &[A::O(src), A::O(surf), A::O(0), A::U(serial)]);
        self.w.flush();
        self.clip.ds = src;
        self.clip.ds_data = data;
        self.clip.ds_action = 0;
        self.clip.ds_win = (w as u32 & (PW_MAX as u32 - 1)) as u8;
        true
    }

    /// the app's drag ended with this effect: E_DRAG_END, the source goes
    fn drag_end(&mut self, effect: u32) {
        let s = self.clip.ds;
        if s == 0 {
            return;
        }
        self.clip.ds = 0;
        for wr in self.clip.writers.iter_mut() {
            if wr.dnd != 0 && wr.fd != 0 {
                sys_close(wr.fd);
                wr.fd = 0;
            }
        }
        self.clip.ds_data.clear();
        self.w.send(s, &DATA_SOURCE_DESTROY, &[]);
        self.w.flush();
        crate::dnd::drag_finished(self.clip.ds_win, effect);
    }
}

// %XX for every byte of a path that is not unreserved (or '/')
fn uri_of(path: &str) -> Vec<u8> {
    let mut u = b"file://".to_vec();
    for &c in path.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'.' | b'_' | b'~') {
            u.push(c);
        } else {
            u.extend_from_slice(alloc::format!("%{c:02X}").as_bytes());
        }
    }
    u
}

// the absolute path of p (relative ones from the working folder)
fn abs_path(p: &str) -> alloc::string::String {
    if p.starts_with('/') {
        return alloc::string::String::from(p);
    }
    let mut d = crate::cwd();
    if !d.ends_with('/') {
        d.push('/');
    }
    d.push_str(p.trim_start_matches("./"));
    d
}
