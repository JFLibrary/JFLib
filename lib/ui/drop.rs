// drop -- (K1) dropped files and other content for apps, the clipboard format probe, the
// headless image clipboard; the command lines of later starts (AF_SINGLE_INSTANCE)
//                                                                       [src/ui/ui_drop.s]
//
//   open_take() -> Vec<Request>     (UIC_OPEN, arg = their count) the working folder and
//                                   arguments of every later start of the app since the last
//                                   call; Request::paths() gives the files, which the app may
//                                   read and write (fs.user); the main window comes forward
//
//   drop_count() -> n               files of the last drop (UIC_DROP, arg = window id; or
//                                   DS_DROP of a drop target, dnd.rs)
//   drop_path(i) -> Option<&str>    path i; valid until the next drop arrives
//   drop_list() -> &[String]        every path of the last drop
//   drop_data() -> &DragData        everything the last drop carried: paths, text, URLs, HTML,
//                                   an image file (PNG, or BMP from a DIB), the app's own kind
//                                   and bytes; drop_types() -> its DT_* kinds, drop_text()
//   drop_pos() -> (x, y)            drop position in DIP of the window it happened in
//   drop_window() -> id             that window's app id (0 = main window)
//   clip_has(fmt) -> bool           the clipboard offers CLIP_TEXT / CLIP_PNG / CLIP_IMAGE
//                                   (headless runs: the in-process clipboard)
// Which kinds a window takes outside its drop targets: drop_accept (dnd.rs, default DT_FILES).
// Internal:
//   drop_receive(ev) -> taken       E_DROP: the platform's pending drop (runtime.rs) onto the
//                                   drop target at the point (true) or for UIC_DROP (false)
//   drop_inject(data)               script "drop": the content of the next E_DROP
//   hclip_img_set(fmt, data)        headless CLIP_PNG (bytes) / CLIP_IMAGE (a CLIPIMG
//                                   record: png then bmp, see clipimg.rs): PNG + BMP copies
//   hclip_img_get(fmt)              CLIP_PNG: the PNG; CLIP_IMAGE: the PNG, else the BMP
//   hclip_img_clear()   hclip_img_has(fmt)

use alloc::string::String;
use alloc::vec::Vec;

use super::consts::*;
use super::Ui;
use plat::{DragData, Event};

/// drop and headless image clipboard state
#[derive(Default)]
pub(crate) struct S {
    pub(crate) list: Vec<String>,
    pub(crate) inj: Option<DragData>,
    pub(crate) hi_png: Option<Vec<u8>>,
    pub(crate) hi_bmp: Option<Vec<u8>>,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) win: u32,
}

impl Ui {
    /// ui_drop_count: files of the last drop
    pub fn drop_count(&self) -> u32 {
        self.dr.list.len() as u32
    }

    /// ui_drop_path(i): path i of the last drop (None when out of range)
    pub fn drop_path(&self, i: u32) -> Option<&str> {
        self.dr.list.get(i as usize).map(|s| s.as_str())
    }

    /// AF_SINGLE_INSTANCE: the command lines of later starts that arrived (UIC_OPEN), oldest
    /// first; each call takes them
    pub fn open_take(&mut self) -> Vec<plat::instance::Request> {
        core::mem::take(&mut self.open_reqs)
    }

    /// E_WAKE: requests of later starts -> their files granted (fs.user), the main window
    /// brought forward, App::command(UIC_OPEN, pending requests)
    pub(crate) fn open_poll(&mut self) {
        let reqs = plat::instance::take();
        if reqs.is_empty() {
            return;
        }
        for p in reqs.iter().flat_map(|r| r.paths()).filter(|p| !p.contains("://")) {
            super::perm::grant_user(&p);
        }
        self.bring_to_front();
        self.open_reqs.extend(reqs);
        let n = self.open_reqs.len() as u32;
        self.cmd_run(UIC_OPEN, n);
    }

    /// ui_drop_list: every path of the last drop
    pub fn drop_list(&self) -> &[String] {
        &self.dr.list
    }

    /// ui_drop_pos: drop position in DIP of the window it happened in
    pub fn drop_pos(&self) -> (i32, i32) {
        (self.dr.x, self.dr.y)
    }

    /// ui_drop_window: the app id of the window of the last drop (0 = main window)
    pub fn drop_window(&self) -> u32 {
        self.dr.win
    }

    /// everything the last drop carried (paths, text, URLs, HTML, image, the app's own kind
    /// and bytes; effects = the effect it was dropped with)
    pub fn drop_data(&self) -> &DragData {
        &self.dnd.dropped
    }

    /// the DT_* kinds of the last drop
    pub fn drop_types(&self) -> u32 {
        self.dnd.dropped.types()
    }

    /// the text of the last drop (None: it had none)
    pub fn drop_text(&self) -> Option<&str> {
        self.dnd.dropped.text.as_deref()
    }

    // drop_install(list, win, x_px, y_px): the new current list
    fn drop_install(&mut self, list: Vec<String>, win: u32, x: i32, y: i32) {
        self.dr.list = list;
        self.dr.win = self.uiw_rec_id(win);
        self.uiw_use(win); // DIP in that window
        let x = if self.rtl { self.wp.rt.fb.w - 1 - x } else { x }; // logical (right-to-left)
        self.dr.x = self.px2dip(x);
        self.dr.y = self.px2dip(y);
        self.uiw_use(0);
    }

    /// ui_drop_accept(ev) -> taken: E_DROP of window ev.win (UI index): the platform's drop
    /// becomes current; a drop target at the point takes it (DS_DROP), else UIC_DROP follows
    pub(crate) fn drop_receive(&mut self, ev: &Event) -> bool {
        let data = if self.headless {
            self.dr.inj.take().unwrap_or_default() // headless: the content of the script's "drop"
        } else {
            plat::drop_data_take()
        };
        super::perm::grant_all(&data.files); // the user dropped them (fs.user)
        let win = ev.win as u32;
        self.drop_install(data.files.clone(), win, ev.x, ev.y);
        let cur = self.uiw_cur;
        self.uiw_use(win);
        let x = if self.rtl { self.wp.rt.fb.w - 1 - ev.x } else { ev.x }; // logical (right-to-left)
        self.uiw_use(cur);
        self.dnd_dropped(win, x, ev.y, data)
    }

    /// ui_drop_inject(data): (script) the next E_DROP gets this content
    pub(crate) fn drop_inject(&mut self, data: DragData) {
        self.dr.inj = Some(data);
    }

    /// ui_clip_has(fmt): the clipboard offers fmt (cheap: no data is transferred)
    pub fn clip_has(&self, fmt: u32) -> bool {
        if !self.headless {
            return plat::clip_has(fmt);
        }
        if fmt == CLIP_TEXT {
            return self.hclip_text_has();
        }
        if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) {
            return self.m.edit.hclip_rich.iter().any(|e| e.0 == fmt);
        }
        self.hclip_img_has(fmt)
    }

    /// hclip_img_clear
    pub(crate) fn hclip_img_clear(&mut self) {
        self.dr.hi_png = None;
        self.dr.hi_bmp = None;
    }

    /// hclip_img_has(fmt): a PNG is stored (CLIP_IMAGE: or a BMP)
    pub(crate) fn hclip_img_has(&self, fmt: u32) -> bool {
        self.dr.hi_png.is_some() || (fmt == CLIP_IMAGE && self.dr.hi_bmp.is_some())
    }

    /// hclip_img_set(fmt, data) -> 0 / -1: CLIP_PNG bytes, or CLIP_IMAGE: a CLIPIMG record
    /// (clipimg::clipimg_record: u64 PNG length, PNG, u64 BMP length, BMP)
    pub(crate) fn hclip_img_set(&mut self, fmt: u32, data: &[u8]) -> i32 {
        match fmt {
            CLIP_PNG => {
                self.hclip_img_set_parts(Some(data), None);
                0
            }
            CLIP_IMAGE => match super::clipimg::clipimg_parse(data) {
                Some((png, bmp)) => {
                    self.hclip_img_set_parts(png, bmp);
                    0
                }
                None => {
                    self.hclip_text_clear();
                    self.hclip_img_clear();
                    -1
                }
            },
            _ => {
                self.hclip_text_clear();
                self.hclip_img_clear();
                -1
            }
        }
    }

    /// the stored image as PNG and / or BMP copies (one content at a time)
    pub(crate) fn hclip_img_set_parts(&mut self, png: Option<&[u8]>, bmp: Option<&[u8]>) {
        self.hclip_text_clear();
        self.hclip_img_clear();
        self.dr.hi_bmp = bmp.map(|b| b.to_vec());
        self.dr.hi_png = png.map(|p| p.to_vec());
    }

    /// hclip_img_get(fmt): CLIP_PNG: the PNG; CLIP_IMAGE: the PNG, else the BMP (copies)
    pub(crate) fn hclip_img_get(&self, fmt: u32) -> Option<Vec<u8>> {
        if fmt != CLIP_PNG && fmt != CLIP_IMAGE {
            return None;
        }
        if let Some(p) = &self.dr.hi_png {
            return Some(p.clone());
        }
        if fmt == CLIP_IMAGE {
            return self.dr.hi_bmp.clone();
        }
        None
    }
}
