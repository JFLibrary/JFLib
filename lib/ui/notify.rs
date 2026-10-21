// notify -- transient feedback: toasts, spinner, indeterminate progress
//
//   toast(kind, title, msg, ms)         queue a notification card (IB_INFO .. IB_ERROR; ms 0
//                                       = 5 s, u32::MAX = until closed); returns its id
//   toast_host()                        draw the queued toasts (bottom right, newest at the
//                                       bottom, above dialogs); call it once per frame after
//                                       the content.  Hovering keeps a toast; its close button
//                                       removes it; expiry wakes the loop (timer UIT_TOAST).
//   toast_dismiss(id) / toast_clear()
//   toasts() -> [(id, kind, title, msg)]   the queued toasts (a notification history)
//   spinner(id, size_dip)               a turning ring while something loads
//   progress_indeterminate(id)          a bar with a running segment (natural 200 x 4 DIP)
// The animations keep the window repainting while they are shown.

use alloc::string::String;
use alloc::vec::Vec;

use crate::consts::*;
use crate::{Sp, Ui};

#[derive(Clone)]
struct Toast {
    id: u32,
    kind: u32,
    title: String,
    msg: String,
    until: u32,
    forever: bool,
}

/// shared state: the toast queue
#[derive(Default)]
pub(crate) struct S {
    toasts: Vec<Toast>,
    next: u32,
}

const TOAST_MAX: usize = 5;

impl Ui {
    /// ui_toast(kind, title, msg, ms) -> toast id
    pub fn toast(&mut self, kind: u32, title: &str, msg: &str, ms: u32) -> u32 {
        let n = &mut self.m.notify;
        n.next = n.next.wrapping_add(1).max(1);
        let id = n.next;
        let forever = ms == u32::MAX;
        let dur = if ms == 0 { 5000 } else { ms };
        let until = self.now_ms.wrapping_add(if forever { 0 } else { dur });
        let n = &mut self.m.notify;
        n.toasts.push(Toast { id, kind: kind & 3, title: String::from(title), msg: String::from(msg), until, forever });
        while n.toasts.len() > TOAST_MAX {
            n.toasts.remove(0);
        }
        if !forever {
            self.timer(UIT_TOAST, dur);
        }
        self.repass();
        id
    }

    /// ui_toast_dismiss(id)
    pub fn toast_dismiss(&mut self, id: u32) {
        self.m.notify.toasts.retain(|t| t.id != id);
        self.repass();
    }

    /// ui_toast_clear
    pub fn toast_clear(&mut self) {
        self.m.notify.toasts.clear();
        self.repass();
    }

    /// the queued toasts: (id, kind, title, message)
    pub fn toasts(&self) -> Vec<(u32, u32, String, String)> {
        self.m.notify.toasts.iter().map(|t| (t.id, t.kind, t.title.clone(), t.msg.clone())).collect()
    }

    /// ui_toast_host: draw the toasts (once per frame, after the content)
    pub fn toast_host(&mut self) {
        let now = self.now_ms;
        // expire
        let before = self.m.notify.toasts.len();
        self.m.notify.toasts.retain(|t| t.forever || (t.until.wrapping_sub(now) as i32) > 0);
        if self.m.notify.toasts.len() != before {
            self.repass();
        }
        if self.m.notify.toasts.is_empty() {
            return;
        }
        let (ww, wh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let w = self.dp(360).min(ww - self.dp(32));
        let h = self.dp(64);
        let gap = self.dp(10);
        let m = self.dp(16);
        let prev_layer = self.layer_set(L_TIP);
        let clip = self.wp.ctx.uiclip;
        let lr = self.wp.ctx.last_rect;
        self.wp.ctx.uiclip = [0, 0, ww, wh];
        let toasts = self.m.notify.toasts.clone();
        let mut y = wh - m - h - self.dp(28); // above a status bar
        let mut next_due = u32::MAX;
        let sh = self.style.shell();
        for t in toasts.iter().rev() {
            let r = [ww - m - w, y, w, h];
            let tid = 0xFFFF_7500u32.wrapping_add(t.id & 0xFF);
            let ev = self.hit_rect(tid, r[0], r[1], r[2], r[3], HB_BLOCK);
            let _ = ev;
            let hovered = self.wp.ctx.hot == tid || (self.mouse().0 >= r[0] && self.mouse().0 < r[0] + r[2] && self.mouse().1 >= r[1] && self.mouse().1 < r[1] + r[3]);
            let mut sp = Sp { flags: t.kind, label: Some(&t.title), label2: Some(&t.msg), ..Sp::default() };
            sp.place(r);
            sh.toast(self, &mut sp);
            let (cx, cs) = (sp.s[0], sp.s[1]);
            let cid = 0xFFFF_7600u32.wrapping_add(t.id & 0xFF);
            let cr = [cx, r[1] + self.dp(10), cs, cs];
            let cev = self.hit_rect(cid, cr[0], cr[1], cr[2], cr[3], HB_FOCUS);
            if cev & EV_HOVER != 0 {
                let c = self.color(T_SUBTLE_HOVER);
                if c & 0xFF00_0000 != 0 {
                    self.cmd_rrect(cr[0], cr[1], cr[2], cr[3], self.dp(4), 0, c, 0, 0);
                }
            }
            let c = self.color(T_TEXT2);
            let px = self.dp(12);
            crate::stock::icon_in(self, crate::keys::IC_CANCEL, cr, px, c);
            self.sem(K_BUTTON, cid, Some(crate::i18n::tr("Schließen")), 0);
            self.wp.ctx.last_rect = r;
            self.wp.ctx.last_state = 0;
            self.sem(K_BOX, tid, Some(&t.title), t.kind as i32);
            if cev & EV_ACT != 0 {
                let id = t.id;
                self.m.notify.toasts.retain(|x| x.id != id);
                self.repass();
            } else if hovered && !t.forever {
                // hovering keeps the toast for another 2 s
                let until = now.wrapping_add(2000);
                if let Some(x) = self.m.notify.toasts.iter_mut().find(|x| x.id == t.id) {
                    if (x.until.wrapping_sub(until) as i32) < 0 {
                        x.until = until;
                    }
                }
            }
            if !t.forever {
                let left = t.until.wrapping_sub(now);
                if (left as i32) > 0 && left < next_due {
                    next_due = left;
                }
            }
            y -= h + gap;
            if y < m {
                break;
            }
        }
        if next_due != u32::MAX {
            self.timer(UIT_TOAST, next_due.max(16));
        }
        self.wp.ctx.uiclip = clip;
        self.wp.ctx.last_rect = lr;
        self.layer_set(prev_layer);
    }

    /// ui_spinner(id, size_dip): a turning ring
    pub fn spinner(&mut self, id: u32, size: i32) {
        let s = self.dp(if size > 0 { size } else { 20 });
        let r = self.place(s, s);
        let phase = if self.anim_off { 0 } else { ((self.now_ms as u64 * 1024 / 900) & 1023) as i32 };
        if !self.anim_off {
            self.animating = true;
        }
        let sh = self.style.shell();
        let mut sp = Sp { value: phase, ..Sp::default() };
        sp.place([r[0] + ((r[2] - s) >> 1), r[1] + ((r[3] - s) >> 1), s, s]);
        sh.spinner(self, &sp);
        self.wp.ctx.last_state = 0;
        self.sem(K_PROGRESS, id, None, -1);
    }

    /// ui_progress_indeterminate(id): a bar with a running segment
    pub fn progress_indeterminate(&mut self, id: u32) {
        let r = self.place(self.dp(200), self.dp(4));
        let phase = if self.anim_off { 300 } else { ((self.now_ms as u64 * 1024 / 1500) & 1023) as i32 };
        if !self.anim_off {
            self.animating = true;
        }
        let sh = self.style.shell();
        let mut sp = Sp { value: phase, ..Sp::default() };
        sp.place(r);
        sh.progress_indeterminate(self, &sp);
        self.wp.ctx.last_state = 0;
        self.sem(K_PROGRESS, id, None, -1);
    }
}
