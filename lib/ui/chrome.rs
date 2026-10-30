// chrome -- window chrome and overlays: content dialog, info bar, status bar, title bar,
// caption buttons, the chrome of secondary windows, edge grips       [src/ui/ui_chrome.s]
//
//   dialog_begin(id, title, open) -> true while open (emit the content, then
//        dialog_buttons and dialog_end; always call dialog_end)
//        WinUI ContentDialog on L_DIALOG: smoke over the window, centred 320..548 DIP
//        wide, title, content; everything below is inert while it is open
//   dialog_buttons(primary, secondary, close) -> DLG_NONE / DLG_PRIMARY / DLG_SECONDARY /
//        DLG_CLOSE -- footer buttons (labels, None = none; primary is the accent button);
//        Enter = primary, Esc = close.  Any result clears the open flag.
//   dialog_end()
//   infobar(id, severity, title, msg) -> EV (EV_ACT: the close button was clicked)
//        IB_INFO / IB_SUCCESS / IB_WARNING / IB_ERROR; the close button has the id id + 1;
//        placed too narrow, the message and then the title are cut with "…"
//   statusbar_begin() / statusbar_end()
//        32 DIP bar (layer fill, top divider) laying out its children left to right,
//        vertically centred, 16 DIP apart (captions: text(TS_CAPTION, ...))
//   edge_grips()   resize handles for frameless windows on Linux (the runtime calls it;
//        also for resizable secondary windows with the library frame)
//   titlebar_begin(title, icon) / titlebar_end()
//        custom title bar row (style: SM_TB_H, SM_TB_H_LARGE with next_flags(ST_LARGE)):
//        icon + title, then the caller's widgets; empty parts are an HB_DRAG region, the end
//        adds the caption buttons (HB_MIN / HB_MAX / HB_CLOSE; with SM_CAP_LEFT the begin puts
//        them first, close / min / max)
// Windows (win.rs):
//   win_chrome_begin(i) / win_chrome_end(i) -> natural content height (DIP)
//        border, title bar (28 DIP tool / 32 DIP dialog: title, close button) and the
//        content box of a secondary window; inside a WF_DIALOG window dialog_buttons puts
//        its footer below the content box (pinned to the bottom)
//
// Ids: dialog parts (id << 3 | k) | 0x80000000; title bar 0xFFFF7400.. (min +1, max +2,
// close +3); status bar 0xFFFF7410, 0xFFFF7411; edge grips 0xFFFF7430..

use alloc::string::String;

use super::consts::*;
use super::keys::*;
use super::layout::BOX_STACK_MAX;
use super::win::{Shared, UWI_BUILDING, WCH_BASE, WCH_BTN, WCH_CLOSE, WCH_CONTENT, WCH_FOOTER};
use super::{Sp, Ui};

const TB_ID: u32 = 0xFFFF_7400;
const TB_MIN_ID: u32 = 0xFFFF_7401;
const TB_MAX_ID: u32 = 0xFFFF_7402;
const TB_CLOSE_ID: u32 = 0xFFFF_7403;
const SB_ID: u32 = 0xFFFF_7410;
const SB_INNER_ID: u32 = 0xFFFF_7411;
#[cfg_attr(windows, allow(dead_code))]
const GRIP_ID: u32 = 0xFFFF_7430; // + 0..7

/// state shared by all windows
#[derive(Default)]
pub(crate) struct S {
    /// width of the next caption button (DIP, 0 = SM_CAP_W)
    cap_w: i32,
    /// id of the first edge grip of the window being built
    #[cfg_attr(windows, allow(dead_code))]
    grip_base: u32,
}

/// per-window state (ui_chrome_wb0: content dialog, title bar, window chrome; swapped by
/// win.rs)
#[derive(Default)]
pub(crate) struct W {
    dlg_clip: [i32; 4],
    dlg_openp: Option<Shared<i32>>,
    pub(crate) dlg_active: bool,
    dlg_nest: u32,
    dlg_id: u32,
    dlg_phase: u32,
    dlg_layer: u32,
    dlg_dis: u32,
    /// the dialog opened in this pass (ui.dialog_sound, sound.rs)
    pub(crate) dlg_fresh: bool,
    tb_state: u32,
    /// This titlebar overlays app content and places the agent badge by the window controls.
    tb_overlay: bool,
    /// dialog_buttons closed the content box of a dialog window
    wc_footer: bool,
}

// a dialog part id: (id << 3 | k) | 0x80000000
fn dlg_part(id: u32, k: u32) -> u32 {
    id.wrapping_mul(8).wrapping_add(k) | 0x8000_0000
}

impl Ui {
    // ------------------------------------------------------------------ content dialog

    /// ui_dialog_begin(id, title, open) -> true if the dialog is open
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn dialog_begin(&mut self, id: u32, title: &str, open: &Shared<i32>) -> bool {
        let _src = self.src_scope();
        if self.wp.chrome.dlg_active {
            self.wp.chrome.dlg_nest += 1; // a dialog inside a dialog is not supported
            return false;
        }
        // slot: aux = open last pass, scrollx = focus before
        if open.get() == 0 {
            // closed: give the focus back once
            let s = self.slot(id);
            if s.aux != 0 {
                s.aux = 0;
                let f = s.scrollx as u32;
                self.wp.ctx.focus = f;
                self.repass();
            }
            return false;
        }
        {
            let c = &mut self.wp.chrome;
            c.dlg_active = true;
            c.dlg_id = id;
            c.dlg_openp = Some(open.clone());
            c.dlg_phase = 1;
            c.dlg_fresh = false;
        }
        let focus = self.wp.ctx.focus;
        let s = self.slot(id);
        if s.aux == 0 {
            // just opened: the keyboard focus moves into the dialog
            s.aux = 1;
            s.scrollx = focus as i32;
            self.wp.ctx.focus = id;
            self.wp.ctx.focus_visible = false;
            self.wp.chrome.dlg_fresh = true;
        }
        self.wp.chrome.dlg_layer = self.layer_set(L_DIALOG);
        self.modal(L_DIALOG);
        // the dialog is laid out and hit tested against the whole window
        self.wp.chrome.dlg_clip = self.wp.ctx.uiclip;
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        self.wp.ctx.uiclip = [0, 0, w, h];
        self.wp.chrome.dlg_dis = self.wp.ctx.disabled_depth;
        self.wp.ctx.disabled_depth = 0;
        // smoke (style)
        let st = self.style;
        let sp = Sp { x: 0, y: 0, w, h, ..Default::default() };
        st.dialog_smoke(self, &sp);
        self.wp.ctx.last_rect = [0, 0, w, h];
        self.hit(dlg_part(id, 1), HB_BLOCK);
        // size: the natural size of the last pass, 320 .. 548 DIP wide
        let (mut dw, nh) = {
            let s = self.slot(id);
            (s.measw, s.meash)
        };
        let min = self.dp(st.metric(SM_DLG_MIN));
        if dw < min {
            dw = min;
        }
        let max = self.dp(st.metric(SM_DLG_MAX));
        if dw > max {
            dw = max;
        }
        let m = self.dp(st.metric(SM_DLG_MARGIN)); // space kept free around it
        if dw > w - m {
            dw = w - m;
        }
        if dw < 1 {
            dw = 1;
        }
        let x = (w - dw) >> 1;
        let y = ((h - nh) >> 1).max(0);
        self.next_rect_px(x, y, dw, 0);
        self.box_begin(id, BOX_V | BOX_BLOCK, 0);
        let r = self.wp.ctx.last_rect;
        // footer height from its last measurement
        let fh = self.slot(dlg_part(id, 4)).meash;
        // paint (style): shadow, surface, border
        let mut sp = Sp { value: fh, ..Default::default() };
        sp.place(r);
        st.dialog_frame(self, &sp);
        // record
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = SS_OPEN as u32;
        self.sem(K_DIALOG, id, Some(title), 0);
        // content: title, then the caller's widgets
        let gap = st.metric(SM_DLG_CONTENT_GAP);
        let flags = st.metric(SM_DLG_CONTENT_BOX) as u32;
        self.box_begin(dlg_part(id, 3), flags, gap);
        let ts = st.metric(SM_DLG_TITLE_TS) as u32;
        self.text(ts, title);
        true
    }

    /// ui_dialog_buttons(primary, secondary, close) -> DLG_* (any result clears the open
    /// flag).  Inside a content dialog: its footer.  Otherwise, inside a WF_DIALOG window:
    /// a footer pinned to the window's bottom; a result clears the window's open flag.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn dialog_buttons(&mut self, primary: Option<&str>, secondary: Option<&str>, close: Option<&str>) -> u32 {
        let _src = self.src_scope();
        if self.wp.chrome.dlg_active {
            let c = &mut self.wp.chrome;
            if c.dlg_nest != 0 || c.dlg_phase != 1 {
                return DLG_NONE;
            }
            c.dlg_phase = 2;
            let id = c.dlg_id;
            self.box_end(); // content
            let r = self.dlg_footer(dlg_part(id, 4), dlg_part(id, 5), primary, secondary, close);
            if r != 0 {
                if let Some(o) = &self.wp.chrome.dlg_openp {
                    o.set(0);
                }
                self.repass();
            }
            return r;
        }
        // a dialog window: close its content box, the footer follows at the bottom
        if self.wp.chrome.wc_footer {
            return DLG_NONE;
        }
        let cur = self.uiw_cur;
        if cur == 0 {
            return DLG_NONE;
        }
        let rec = &self.m.win.recs[(cur as usize) & (WIN_MAX - 1)];
        if rec.flags & WF_DIALOG == 0 || rec.state & UWI_BUILDING == 0 {
            return DLG_NONE;
        }
        let openp = rec.openp.clone();
        self.wp.chrome.wc_footer = true;
        let ids = cur << 4 | WCH_BASE;
        self.wc_pop_to(ids + WCH_CONTENT);
        let r = self.dlg_footer(ids + WCH_FOOTER, ids + WCH_BTN, primary, secondary, close);
        if r != 0 {
            if let Some(o) = openp {
                o.set(0);
            }
            self.repass();
        }
        r
    }

    // dlg_footer(footer_id, button_id, primary, secondary, close) -> DLG_*: the button row
    // of a dialog (ids button_id .. +2); Enter = primary, Esc = close
    fn dlg_footer(&mut self, fid: u32, bid: u32, p: Option<&str>, s: Option<&str>, c: Option<&str>) -> u32 {
        let st = self.style;
        let gap = st.metric(SM_DLG_FOOTER_GAP);
        let flags = st.metric(SM_DLG_FOOTER_BOX) as u32;
        self.box_begin(fid, flags, gap);
        let order = st.metric(SM_DLG_BTN_ORDER);
        let mut r = DLG_NONE;
        if order == 0 {
            // a single button takes the right half
            let n = p.is_some() as u32 + s.is_some() as u32 + c.is_some() as u32;
            if n == 1 {
                self.spacer(-1);
            }
            if let Some(l) = p {
                self.next_size(-1, 0);
                if self.accent_button(bid, l) & EV_ACT != 0 {
                    r = DLG_PRIMARY;
                }
            }
            if let Some(l) = s {
                self.next_size(-1, 0);
                if self.button(bid + 1, l) & EV_ACT != 0 {
                    r = DLG_SECONDARY;
                }
            }
            if let Some(l) = c {
                self.next_size(-1, 0);
                if self.button(bid + 2, l) & EV_ACT != 0 {
                    r = DLG_CLOSE;
                }
            }
        } else {
            // SM_DLG_BTN_ORDER 1 (macOS): right-aligned at their natural width, primary
            // last; 2 (KDE): right-aligned, primary first
            self.spacer(-1);
            if order != 2 {
                if let Some(l) = c {
                    if self.button(bid + 2, l) & EV_ACT != 0 {
                        r = DLG_CLOSE;
                    }
                }
                if let Some(l) = s {
                    if self.button(bid + 1, l) & EV_ACT != 0 {
                        r = DLG_SECONDARY;
                    }
                }
                if let Some(l) = p {
                    if self.accent_button(bid, l) & EV_ACT != 0 {
                        r = DLG_PRIMARY;
                    }
                }
            } else {
                if let Some(l) = p {
                    if self.accent_button(bid, l) & EV_ACT != 0 {
                        r = DLG_PRIMARY;
                    }
                }
                if let Some(l) = s {
                    if self.button(bid + 1, l) & EV_ACT != 0 {
                        r = DLG_SECONDARY;
                    }
                }
                if let Some(l) = c {
                    if self.button(bid + 2, l) & EV_ACT != 0 {
                        r = DLG_CLOSE;
                    }
                }
            }
        }
        self.box_end(); // footer
        // keyboard: Enter = primary, Esc = close (keys a focused widget did not take)
        if r != 0 {
            return r;
        }
        if p.is_some() && self.key(VK_RETURN) {
            return DLG_PRIMARY;
        }
        if self.key(VK_ESCAPE) {
            return DLG_CLOSE;
        }
        DLG_NONE
    }

    /// ui_dialog_end
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn dialog_end(&mut self) {
        let _src = self.src_scope();
        if self.wp.chrome.dlg_nest != 0 {
            self.wp.chrome.dlg_nest -= 1;
            return;
        }
        if !self.wp.chrome.dlg_active {
            return;
        }
        if self.wp.chrome.dlg_phase == 1 {
            // no footer: Esc still closes
            if self.key(VK_ESCAPE) {
                if let Some(o) = &self.wp.chrome.dlg_openp {
                    o.set(0);
                }
                self.repass();
            }
            self.box_end(); // content
        }
        self.box_end(); // outer
        let c = &self.wp.chrome;
        let (clip, dis, layer) = (c.dlg_clip, c.dlg_dis, c.dlg_layer);
        self.wp.ctx.uiclip = clip;
        self.wp.ctx.disabled_depth = dis;
        self.layer_set(layer);
        self.wp.chrome.dlg_active = false;
    }

    // ------------------------------------------------------------------ info bar

    /// ui_infobar(id, severity, title, msg) -> EV (EV_ACT when the close button was clicked)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn infobar(&mut self, id: u32, severity: u32, title: &str, msg: &str) -> u32 {
        let _src = self.src_scope();
        let sev = severity & 3;
        let st = self.style;
        // measure (style)
        let mut sp = Sp { flags: sev, label: Some(title), label2: Some(msg), ..Default::default() };
        let (w, h) = st.infobar_size(self, &mut sp);
        self.place(w, h);
        let r = self.wp.ctx.last_rect;
        // narrower than the texts: the message cut with "…" (left out where little of it
        // would stay), then the title; the style measures them again (s[0] / s[1])
        let fit = if w > r[2] { Some(self.infobar_fit(title, msg, sp.s[0], sp.s[1], w - r[2])) } else { None };
        if let Some((t, m)) = &fit {
            sp.label = Some(t);
            sp.label2 = if m.is_empty() { None } else { Some(m) };
            st.infobar_size(self, &mut sp);
        }
        // paint (style): card, icon, title, message; it reports the close button's place
        sp.place(r);
        st.infobar(self, &mut sp);
        let (cx, cs) = (sp.s[2], sp.s[3]);
        self.wp.ctx.last_rect = [cx, ((r[3] - cs) >> 1) + r[1], cs, cs];
        let ev = self.behave(id.wrapping_add(1), HB_FOCUS);
        // paint (style): the close button
        sp.place(self.wp.ctx.last_rect);
        sp.ev = ev;
        st.infobar_close(self, &sp);
        if ev & EV_FOCUS != 0 && self.wp.ctx.focus_visible {
            let lr = self.wp.ctx.last_rect;
            st.focus_rect(self, lr[0], lr[1], lr[2], lr[3]);
        }
        self.sem(K_BUTTON, id.wrapping_add(1), Some("Close"), 0);
        // record of the bar: label = title, value = severity
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = 0;
        self.sem(K_BOX, id, Some(title), sev as i32);
        ev & EV_ACT
    }

    // infobar_fit(title, msg, title width, message width, px too wide) -> (title, message):
    // the message (TS_BODY) gives way first, then the title (TS_BODY_STRONG); the styles
    // measure them in these text styles
    fn infobar_fit(&mut self, title: &str, msg: &str, tw: i32, mw: i32, over: i32) -> (String, String) {
        let keep = mw - over;
        if !msg.is_empty() && keep >= self.dp(32) {
            self.ts_font(TS_BODY);
            return (String::from(title), self.ellipsize(msg.as_bytes(), keep).0);
        }
        let over = over - mw;
        if over <= 0 {
            return (String::from(title), String::new());
        }
        self.ts_font(TS_BODY_STRONG);
        (self.ellipsize(title.as_bytes(), (tw - over).max(1)).0, String::new())
    }

    // ------------------------------------------------------------------ status bar

    /// ui_statusbar_begin: a 32 DIP bar; call it last in a vertical layout (after a fill)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn statusbar_begin(&mut self) {
        let _src = self.src_scope();
        let st = self.style;
        let h = st.metric(SM_SB_H); // height (DIP)
        self.next_size(0, h);
        self.box_begin(SB_ID, BOX_H, 0);
        let r = self.wp.ctx.last_rect;
        let mut sp = Sp::default();
        sp.place(r);
        st.statusbar(self, &sp);
        // content row, inset at both sides
        let pad = self.dp(st.metric(SM_SB_PAD));
        let mut w = r[2] - pad - pad;
        if w <= 0 {
            w = 1;
        }
        self.next_rect_px(r[0] + pad, r[1], w, r[3]);
        let gap = st.metric(SM_SB_GAP); // gap between the items (DIP)
        self.box_begin(SB_INNER_ID, BOX_H | BOX_XCENTER, gap);
    }

    /// ui_statusbar_end
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn statusbar_end(&mut self) {
        let _src = self.src_scope();
        self.box_end();
        self.box_end();
    }

    // ------------------------------------------------------------------ title bar

    /// ui_titlebar_begin(title, icon): a title bar row (icon 0 = none)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn titlebar_begin(&mut self, title: &str, icon: u32) {
        let _src = self.src_scope();
        self.titlebar_begin_opt(Some(title), icon);
    }

    /// ui_titlebar_begin with title 0: no title text (the record has no label)  // pn-core
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn titlebar_begin_opt(&mut self, title: Option<&str>, icon: u32) {
        let _src = self.src_scope();
        self.titlebar_begin_inner(title, icon, false);
    }

    /// Overlay native window controls and a drag region without painting a titlebar surface.
    /// Use `next_rect_px` to place it over the empty top edge of a file/start screen.
    /// The agent indicator remains visible beside the caption buttons when control is enabled.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn titlebar_begin_overlay(&mut self, title: Option<&str>, icon: u32) {
        let _src = self.src_scope();
        self.titlebar_begin_inner(title, icon, true);
    }

    fn titlebar_begin_inner(&mut self, title: Option<&str>, icon: u32, overlay: bool) {
        let st = self.style;
        self.wp.chrome.tb_overlay = overlay;
        let f = self.take_flags();
        let h = st.metric(if f & ST_LARGE != 0 { SM_TB_H_LARGE } else { SM_TB_H });
        self.next_size(0, h);
        self.box_begin(TB_ID, BOX_H | BOX_XCENTER, 0);
        // window state (a headless run looks active)
        self.wp.chrome.tb_state = if self.headless { plat::WST_ACTIVE } else { plat::window_state() };
        let tbs = self.wp.chrome.tb_state;
        // background (style)
        let mut sp = Sp { value: (tbs & plat::WST_ACTIVE != 0) as i32, ..Default::default() };
        sp.place(self.wp.ctx.last_rect);
        if !overlay { st.window_titlebar(self, &sp); }
        // the whole bar is a drag region; widgets placed on it later win the hit test
        let ev = self.behave(TB_ID, HB_DRAG);
        if !self.headless {
            let mut release = false;
            if ev & EV_DOUBLE != 0 {
                plat::window_cmd(if tbs & plat::WST_MAXIMIZED != 0 { plat::WCMD_RESTORE } else { plat::WCMD_MAXIMIZE });
                release = true;
            } else if self.wp.ctx.active == TB_ID && self.wp.ctx.pressed & (1 << BTN_LEFT) != 0 {
                plat::begin_move();
                release = true;
            }
            if release {
                // the OS owns the pointer now: forget the press
                self.wp.ctx.active = 0;
                self.wp.ctx.mbtn &= !(1 << BTN_LEFT);
            }
            // Linux: a right press on the bar opens the window manager's window menu (the
            // system menu of Windows' own caption)
            #[cfg(target_os = "linux")]
            if self.wp.ctx.ractive == TB_ID && self.wp.ctx.pressed & (1 << BTN_RIGHT) != 0 {
                plat::window_menu(self.wp.ctx.mx, self.wp.ctx.my);
                self.wp.ctx.ractive = 0;
                self.wp.ctx.mbtn &= !(1 << BTN_RIGHT);
            }
        }
        self.wp.ctx.last_state = 0;
        self.sem(K_TITLEBAR, TB_ID, title, 0);
        // caption buttons at the left end (SM_CAP_LEFT: macOS)
        if st.metric(SM_CAP_LEFT) != 0 {
            self.spacer(st.metric(SM_TB_PAD));
            self.tb_cap_close();
            self.tb_cap_min();
            self.tb_cap_max();
        }
        // icon and title
        self.spacer(st.metric(SM_TB_PAD)); // space before the icon / title (DIP)
        if icon != 0 {
            let s = st.metric(SM_TB_ICON); // icon size (DIP; 0 = the style shows none)
            if s != 0 {
                self.icon(icon, s);
                self.spacer(st.metric(SM_TB_PAD));
            }
        }
        if self.agent_enabled() && !overlay {
            self.next_size(82, 24);
            if self.button(0xFFFF_7440, if self.agent_connected() { "Agent ●" } else { "Agent ○" }) & EV_ACT != 0 {
                self.set_agent_enabled(false);
            }
            self.tooltip(if self.agent_connected() { "Agent verbunden · Zum Abschalten klicken" } else { "Agentensteuerung aktiv · Zum Abschalten klicken" });
            self.spacer(8);
        }
        let Some(title) = title else { return };
        let mut ts = st.metric(SM_TB_TITLE_TS) as u32;
        if tbs & plat::WST_ACTIVE == 0 {
            ts |= TX_DISABLED;
        }
        self.text(ts, title);
    }

    /// ui_titlebar_end: caption buttons at the right end (unless the style puts them at the
    /// left: SM_CAP_LEFT)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn titlebar_end(&mut self) {
        let _src = self.src_scope();
        let st = self.style;
        if self.wp.chrome.tb_overlay {
            self.spacer(-1);
            if self.agent_enabled() {
                self.next_size(82, 24);
                if self.button(0xFFFF_7440, if self.agent_connected() { "Agent ●" } else { "Agent ○" }) & EV_ACT != 0 {
                    self.set_agent_enabled(false);
                }
                self.tooltip(if self.agent_connected() { "Agent verbunden · Zum Abschalten klicken" } else { "Agentensteuerung aktiv · Zum Abschalten klicken" });
                self.spacer(8);
            }
        }
        if st.metric(SM_CAP_LEFT) == 0 {
            if !self.wp.chrome.tb_overlay { self.spacer(-1); }
            self.tb_cap_min();
            self.tb_cap_max();
            self.tb_cap_close();
        }
        self.box_end();
    }

    // tb_cap_min / tb_cap_max / tb_cap_close: one caption button of the title bar and its
    // window command
    fn tb_cap_min(&mut self) {
        if self.cap_button(TB_MIN_ID, IC_CHROME_MIN, HB_MIN, "Minimize") & EV_ACT != 0 {
            self.tb_cmd(plat::WCMD_MINIMIZE);
        }
    }

    fn tb_cap_max(&mut self) {
        let max = self.wp.chrome.tb_state & plat::WST_MAXIMIZED != 0;
        let (ic, l) = if max { (IC_CHROME_RESTORE, "Restore") } else { (IC_CHROME_MAX, "Maximize") };
        if self.cap_button(TB_MAX_ID, ic, HB_MAX, l) & EV_ACT != 0 {
            let max = self.wp.chrome.tb_state & plat::WST_MAXIMIZED != 0;
            self.tb_cmd(if max { plat::WCMD_RESTORE } else { plat::WCMD_MAXIMIZE });
        }
    }

    fn tb_cap_close(&mut self) {
        if self.cap_button(TB_CLOSE_ID, IC_CHROME_CLOSE, HB_CLOSE, "Close") & EV_ACT != 0 {
            self.tb_cmd(plat::WCMD_CLOSE);
        }
    }

    // tb_cmd(cmd): plat::window_cmd, except in headless runs
    fn tb_cmd(&mut self, cmd: u32) {
        if !self.headless {
            plat::window_cmd(cmd);
        }
    }

    // cap_button(id, icon, hbflags, label) -> EV: a caption button over the full height of
    // the title bar (close: red hover with a white glyph)
    fn cap_button(&mut self, id: u32, icon: u32, hb: u32, label: &str) -> u32 {
        let st = self.style;
        let bh = self.box_rect()[3]; // bar height
        let w = if self.m.chrome.cap_w != 0 { self.m.chrome.cap_w } else { st.metric(SM_CAP_W) };
        let w = self.dp(w);
        self.place(w, bh);
        let ev = self.behave(id, hb);
        // paint (style)
        let active = (self.wp.chrome.tb_state & plat::WST_ACTIVE != 0) as i32;
        let mut sp = Sp { ev, flags: hb, icon, value: active, ..Default::default() };
        sp.place(self.wp.ctx.last_rect);
        st.caption_button(self, &sp);
        self.sem(K_BUTTON, id, Some(label), 0);
        ev
    }

    // ------------------------------------------------------------------ window chrome

    /// win_chrome_begin(i): (window i is live and its pass has begun) the border, the title
    /// bar (drag region with the title, close button) and the content box of a secondary
    /// window
    pub(crate) fn win_chrome_begin(&mut self, i: u32) {
        let st = self.style;
        let iu = (i as usize) & (WIN_MAX - 1);
        let (flags, tbh) = (self.m.win.recs[iu].flags, self.m.win.recs[iu].tbh);
        let ids = i << 4 | WCH_BASE; // chrome ids
        self.wp.chrome.wc_footer = false;
        // active look: tool windows share the app's activation, others their own
        let mut tbs = 0;
        if self.uiw_app_active() {
            tbs = plat::WST_ACTIVE;
            if flags & WF_TOOL == 0 && i != self.m.win.active {
                tbs = 0;
            }
        }
        self.wp.chrome.tb_state = tbs;
        let b = self.uiw_border();
        if b != 0 {
            // paint (style): the window border
            let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
            let sp = Sp { x: 0, y: 0, w, h, value: b, ..Default::default() };
            st.window_border(self, &sp);
        }
        if tbh != 0 {
            self.next_size(0, tbh);
            self.box_begin(ids, BOX_H | BOX_XCENTER, 0);
            // background (style)
            let mut sp = Sp { value: (tbs & plat::WST_ACTIVE != 0) as i32, ..Default::default() };
            sp.place(self.wp.ctx.last_rect);
            st.window_titlebar(self, &sp);
            // the whole bar is a drag region (the platform moves the window: Windows through
            // WM_NCHITTEST, Linux with a press here)
            self.behave(ids, HB_DRAG);
            if !self.headless && self.wp.ctx.active == ids && self.wp.ctx.pressed & (1 << BTN_LEFT) != 0 {
                plat::begin_move(); // (the live window: plat::win_use)
                self.wp.ctx.active = 0; // the OS owns the pointer now
                self.wp.ctx.mbtn &= !(1 << BTN_LEFT);
            }
            let title = alloc::string::String::from(self.uiw_title(i));
            self.wp.ctx.last_state = 0;
            self.sem(K_TITLEBAR, ids, Some(&title), 0);
            self.spacer(st.metric(SM_WC_TITLE_PAD)); // space before the title (DIP)
            // close button at the left end (SM_CAP_LEFT: macOS)
            let mut capw = 0; // its width (DIP)
            let left = st.metric(SM_CAP_LEFT);
            if left != 0 {
                capw = self.wc_close_btn(ids, i);
                if capw != 0 {
                    self.spacer(st.metric(SM_WC_TITLE_PAD));
                }
            }
            // the title fills the bar and is cut with "…" in narrow windows, so the close
            // button always stays inside (U3)
            self.next_size(SIZE_FILL, 0);
            let mut ts = st.metric(SM_TB_TITLE_TS) as u32 | TX_ELLIPSIS;
            if tbs & plat::WST_ACTIVE == 0 {
                ts |= TX_DISABLED;
            }
            self.text(ts, &title);
            self.spacer(st.metric(SM_WC_TITLE_GAP)); // space after the title (DIP)
            if left == 0 {
                self.wc_close_btn(ids, i);
            } else if capw != 0 {
                // left close button: the same space at the right keeps the title centred
                self.spacer(capw);
                self.spacer(st.metric(SM_WC_TITLE_PAD));
            }
            self.box_end();
        }
        // content
        self.next_size(0, -1);
        if flags & WF_DIALOG != 0 {
            let gap = st.metric(SM_DLG_CONTENT_GAP);
            let bf = st.metric(SM_DLG_CONTENT_BOX) as u32 | BOX_FIXED;
            self.box_begin(ids + WCH_CONTENT, bf, gap);
            // dialog: the content area on the layer colour, the footer on the background
            let mut sp = Sp::default();
            sp.place(self.wp.ctx.last_rect);
            st.window_dialog_fill(self, &sp);
        } else {
            self.box_begin(ids + WCH_CONTENT, BOX_V | BOX_FIXED, 0);
        }
    }

    // wc_close_btn(ids, i) -> button width (DIP; 0 = none, WF_NOCLOSE): the close button of
    // a secondary window's title bar and its command
    fn wc_close_btn(&mut self, ids: u32, i: u32) -> i32 {
        let (flags, id) = {
            let r = &self.m.win.recs[(i as usize) & (WIN_MAX - 1)];
            (r.flags, r.id)
        };
        if flags & WF_NOCLOSE != 0 {
            return 0;
        }
        let st = self.style;
        let w = st.metric(if flags & WF_TOOL != 0 { SM_CAP_W_TOOL } else { SM_CAP_W });
        self.m.chrome.cap_w = w;
        let ev = self.cap_button(ids + WCH_CLOSE, IC_CHROME_CLOSE, HB_CLOSE, "Close");
        self.m.chrome.cap_w = 0;
        if ev & EV_ACT != 0 {
            self.cmd_run(UIC_WINCLOSE, id);
        }
        w
    }

    /// win_chrome_end(i) -> natural height of the content (DIP, content box + footer):
    /// closes the content box (unless dialog_buttons did)
    pub(crate) fn win_chrome_end(&mut self, i: u32) -> i32 {
        let ids = i << 4 | WCH_BASE;
        let footer = self.wp.chrome.wc_footer;
        if !footer {
            self.wc_pop_to(ids + WCH_CONTENT);
        }
        let mut h = self.slot(ids + WCH_CONTENT).meash;
        if footer {
            h += self.slot(ids + WCH_FOOTER).meash;
        }
        self.px2dip(h)
    }

    // wc_pop_to(id): close boxes up to and including the box `id` (boxes the app left open
    // inside it are closed with a warning); nothing when it is not open
    fn wc_pop_to(&mut self, id: u32) {
        for _ in 0..BOX_STACK_MAX {
            let b = self.box_id();
            if b == id {
                self.box_end();
                return;
            }
            if b == self.wp.lay.root_id {
                return;
            }
            self.warn("ui_box_begin without ui_box_end in a window");
            self.box_end();
        }
    }

    // ------------------------------------------------------------------ edge grips

    /// ui_edge_grips: invisible resize handles along the window edges for windows without a
    /// system frame on Linux (AF_CUSTOM_TITLEBAR or the library's CSD title bar): a press
    /// starts plat::begin_resize with the edge; the pointer shows the matching arrows.
    /// Called by the runtime after App::frame (so the grips win the hit test).  Nothing on
    /// Windows (WM_NCHITTEST does it there), in headless runs or while maximized /
    /// fullscreen.
    #[cfg(windows)]
    pub(crate) fn edge_grips(&mut self) {}

    #[cfg(not(windows))]
    pub(crate) fn edge_grips(&mut self) {
        if self.headless {
            return;
        }
        // ids: the main window's, else in the window's chrome range
        let i = self.uiw_cur & (WIN_MAX as u32 - 1);
        self.m.chrome.grip_base = if i == 0 { GRIP_ID } else { (i << 4) + 8 | WCH_BASE };
        if plat::window_state() & (plat::WST_MAXIMIZED | plat::WST_FULLSCREEN) != 0 {
            return;
        }
        let st = self.style;
        let t = self.dp(st.metric(SM_GRIP)); // edge handle thickness
        let c = self.dp(st.metric(SM_GRIP_CORNER)); // corner handle size
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        use plat::{EDGE_BOTTOM, EDGE_TOP};
        // right-to-left: the logical left edge is the window's right one
        let (left, right) = if self.rtl { (plat::EDGE_RIGHT, plat::EDGE_LEFT) } else { (plat::EDGE_LEFT, plat::EDGE_RIGHT) };
        let (nwse, nesw) = if self.rtl { (CUR_SIZE_NESW, CUR_SIZE_NWSE) } else { (CUR_SIZE_NWSE, CUR_SIZE_NESW) };
        // edges: top, bottom, left, right (between the corners)
        self.grip(0, EDGE_TOP, CUR_SIZE_NS, c, 0, w - c - c, t);
        self.grip(1, EDGE_BOTTOM, CUR_SIZE_NS, c, h - t, w - c - c, t);
        self.grip(2, left, CUR_SIZE_WE, 0, c, t, h - c - c);
        self.grip(3, right, CUR_SIZE_WE, w - t, c, t, h - c - c);
        // corners (squares)
        self.grip(4, EDGE_TOP | left, nwse, 0, 0, c, c);
        self.grip(5, EDGE_TOP | right, nesw, w - c, 0, c, c);
        self.grip(6, EDGE_BOTTOM | left, nesw, 0, h - c, c, c);
        self.grip(7, EDGE_BOTTOM | right, nwse, w - c, h - c, c, c);
    }

    // grip(k, edge, cursor, x, y, w, h): one handle; a press resizes
    #[cfg(not(windows))]
    #[allow(clippy::too_many_arguments)]
    fn grip(&mut self, k: u32, edge: u32, cur: u32, x: i32, y: i32, w: i32, h: i32) {
        let id = k.wrapping_add(self.m.chrome.grip_base);
        self.hit_rect(id, x, y, w, h, (cur & 0xFF) << HB_CURSOR_SHIFT);
        if self.wp.ctx.active == id && self.wp.ctx.pressed & (1 << BTN_LEFT) != 0 {
            plat::begin_resize(edge);
            // the window manager owns the pointer now: forget the press
            self.wp.ctx.active = 0;
            self.wp.ctx.mbtn &= !(1 << BTN_LEFT);
        }
    }
}
