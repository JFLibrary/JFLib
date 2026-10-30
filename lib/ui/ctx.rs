// ctx -- UI context: input state, passes, hit testing, hot / active / focus, state slots and
// animations, deferred commands and timers, semantic records, clip, layers, modal,
// disabled regions, warnings                                            [src/ui/ui_ctx.s]
//
// App-facing:
//   now() -> ms                     milliseconds (virtual clock in headless runs)
//   key(vk) -> bool                 vk pressed this frame and not yet consumed (consumes it)
//   key_peek(vk) / key_down(vk)     the same without consuming / while the key is held
//   mods() -> MOD_*   mouse() -> (x, y) px (-1 outside)   pen() -> (pressure, PTR_*)
//   focus(id) / focused() -> id     give / ask the keyboard focus
//   cmd_run(id, arg)                queue a command for App::command (runs after the frame)
//   timer(id, ms)                   one-shot timer -> App::command(UIC_TIMER, id)
//   anim(id, target, ms) -> value   animated value 0..256 towards target
//   warn(msg)                       "UIWARN f<frame>: ..." on stderr; scripts and jfgui_DEBUG
//                                   also warn once about an id two widgets of one pass share
//                                   (compound widgets take id + 1 .. or (id << 3 | k) parts)
// Widget building blocks:
//   behave(id, hbflags) -> EV_*     hit box on last_rect + pointer / keyboard behaviour
//   hit(id, hbflags)                hit box only
//   hit_rect(id, x, y, w, h, hb)    behave on an explicit rectangle (px)
//   slot(id) -> &mut Slot           persistent per-id state record
//   sem(kind, id, label, value)     semantic record (state from the last behave)
//   clip_push(x, y, w, h) / clip_pop()   clip drawing and hit boxes (px)
//   layer_set(l) -> previous        modal(layer)   disabled_push() / disabled_pop()
// Runtime (runtime.rs, win.rs): frame_begin / frame_end (a pass over all windows),
// uiw_pass_begin / uiw_pass_end (one window), input_event, input_clear_edges, hot_update.

use alloc::string::String;
use alloc::vec::Vec;

use super::consts::*;
use super::keys::*;
use super::Ui;
use plat::Event;

// The queues and stacks of a pass grow as needed (Vec: amortised growth, the capacity stays,
// so the steady state allocates nothing).  Their ceilings only guard against runaway memory
// (an app queueing in a loop); what a ceiling holds back is never dropped silently:
// limit_hit counts it (Ui::limits) and warns the first time (UIWARN; scripts: expect_warn).

/// keys one pass sees (the platform gives every key a pass of its own; more come from a tool
/// window handing on what its widgets left, or the script command `keys`)
pub(crate) const KEYQ_MAX: usize = 1024;
/// typed codepoints a pass sees at most (one input method commit); more waits for the next
/// passes (Ui::text_more), nothing typed is dropped
pub(crate) const TEXTQ_MAX: usize = 4096;
/// pointer positions of one pass for strokes (a 1 kHz pen during a long frame)
pub(crate) const PTS_MAX: usize = 1 << 16;
/// commands waiting for App::command (the UIC_TIMER of due timers among them)
pub(crate) const CMDQ_MAX: usize = 1 << 16;
/// timers waiting at once (distinct ids, the library's caret, tool tip, toast ... included)
pub(crate) const TIMERS_MAX: usize = 4096;
/// clip_push levels inside each other (the render's clip stack has no limit)
pub(crate) const UICLIP_MAX: usize = 1024;
/// hit boxes (and drop targets) of one window in one pass
pub(crate) const HITS_MAX: usize = 1 << 16;
/// semantic records of one pass (all windows) and the bytes of their labels
pub(crate) const SEMS_MAX: usize = 1 << 16;
pub(crate) const SEMTEXT_MAX: usize = 4 << 20;

/// the ceilings limit_hit reports (Ui::limits[LIM_*]); fixed ones say next to their constant
/// why they stay fixed
pub(crate) const LIM_KEYS: usize = 0;
pub(crate) const LIM_CMDS: usize = 1;
pub(crate) const LIM_TIMERS: usize = 2;
pub(crate) const LIM_POINTS: usize = 3;
pub(crate) const LIM_HITS: usize = 4;
pub(crate) const LIM_ZONES: usize = 5;
pub(crate) const LIM_SEMS: usize = 6;
pub(crate) const LIM_SEMTEXT: usize = 7;
pub(crate) const LIM_CLIP: usize = 8;
pub(crate) const LIM_WINNEST: usize = 9;
pub(crate) const LIM_POPUP_SLOTS: usize = 10;
pub(crate) const LIM_POPUPS: usize = 11;
pub(crate) const LIM_FLYOUTS: usize = 12;
pub(crate) const LIM_MENU_ROWS: usize = 13;
pub(crate) const LIM_MENUBAR: usize = 14;
pub(crate) const LIM_SCROLLS: usize = 15;
pub(crate) const LIM_LISTS: usize = 16;
pub(crate) const LIM_EXPANDERS: usize = 17;
pub(crate) const LIM_WRAP: usize = 18;
pub(crate) const LIM_N: usize = 19;
const LIM_TEXT: [&str; LIM_N] = [
    "more than 1024 keys in one pass: the rest dropped",
    "more than 65536 commands waiting: the rest dropped",
    "more than 4096 timers at once: new ones dropped",
    "more than 65536 pointer positions in one pass: the rest dropped",
    "more than 65536 hit boxes in one window pass: the rest cannot be hit",
    "more than 65536 drop targets in one window pass: the rest take no drops",
    "more than 65536 widget records in one pass: the rest missing (dump, expect, screen readers)",
    "more than 4 MiB of record labels in one pass: the rest without labels",
    "clip_push nested more than 1024 deep: deeper levels clip no hit boxes",
    "window_begin nested more than 16 deep: deeper windows not shown",
    "more than 4 popups outside their windows: the rest drawn clipped to the window",
    "popups nested more than 6 deep: the deepest one replaced",
    "flyouts nested more than 8 deep: deeper ones not shown",
    "a menu with more than 250 rows: the rest not shown",
    "a menu bar with more than 16 menus: the rest not shown",
    "scroll views nested more than 8 deep: deeper ones share one record",
    "lists nested more than 4 deep: deeper ones share one record",
    "expanders nested more than 16 deep: deeper ones lose their content box",
    "a wrapped text of more than 65536 lines: the rest not shown",
];

/// hit record of a pass (clipped, physical px; x1 / y1 exclusive)  [HR_*]
#[derive(Clone, Copy, Default)]
pub(crate) struct Hit {
    pub(crate) id: u32,
    /// Additional region emitted by the same logical widget call (e.g. a text line).
    pub(crate) continuation: bool,
    /// HB_* (cursor in bits 12..15)
    pub(crate) flags: u16,
    pub(crate) layer: u8,
    #[allow(dead_code)]
    pub(crate) kind: u8,
    /// keyboard (kbnav.rs): the focus group (0 = none) and the pane (0 = the window's root)
    pub(crate) group: u32,
    pub(crate) pane: u32,
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) x1: i32,
    pub(crate) y1: i32,
}

/// semantic record (dump / expect)  [SR_*]
#[derive(Clone, Default)]
pub struct Sem {
    pub id: u32,
    pub kind: u16,
    /// SS_*
    pub state: u16,
    /// x, y, w, h physical px
    pub rect: [i32; 4],
    pub label: Option<String>,
    pub value: i32,
    pub max: i32,
    /// the lower end of a slider's range (accessibility)
    pub min: i32,
    pub layer: u8,
    pub depth: u8,
    /// window index (0 = the main window)
    pub win: u8,
}

/// persistent per-id state (slot)  [SL_*]
#[derive(Clone, Copy, Default)]
pub struct Slot {
    pub seen: u32,
    pub afrom: u16,
    pub ato: u16,
    pub at0: u32,
    pub adur: u32,
    pub measw: i32,
    pub meash: i32,
    pub fixed: i32,
    pub wsum: i32,
    pub scrollx: i32,
    pub scrolly: i32,
    pub aux: i32,
}

/// ease(curve, t) -> the eased progress: t and the result in 0..256 (EASE_OUT_BACK overshoots
/// above 256 on the way)
pub fn ease(curve: u32, t: i32) -> i32 {
    let t = t.clamp(0, 256);
    let u = 256 - t;
    match curve {
        EASE_LINEAR => t,
        EASE_IN => (t * t) >> 8,
        EASE_IN_OUT => {
            // 4 t^3 below the middle, 1 - (-2 t + 2)^3 / 2 above it
            if t < 128 {
                ((4 * t * t >> 8) * t) >> 8
            } else {
                let v = 2 * u;
                256 - (((v * v >> 8) * v) >> 9)
            }
        }
        EASE_OUT_CUBIC => 256 - (((u * u >> 8) * u) >> 8),
        EASE_OUT_BACK => {
            // 1 + c3 (t - 1)^3 + c1 (t - 1)^2 with c1 = 1.70158, c3 = c1 + 1 (in 1/256)
            let x = t - 256;
            let x2 = x * x >> 8;
            let x3 = x2 * x >> 8;
            256 + (x3 * 691 >> 8) + (x2 * 436 >> 8)
        }
        // EASE_OUT: 1 - (1 - t)^2 (anim's curve)
        _ => 256 - ((u * u) >> 8),
    }
}

/// the per-window part of the context (ui_ctx_wb0 .. ui_ctx_wb1)
pub(crate) struct W {
    pub(crate) last_rect: [i32; 4],
    /// x0, y0, x1, y1
    pub(crate) uiclip: [i32; 4],
    pub(crate) uiclip_stack: Vec<[i32; 4]>,
    /// clip_push levels beyond UICLIP_MAX (their clip_pop restore nothing)
    pub(crate) uiclip_over: u32,
    /// two lists: current / previous pass (hit_cur selects the current one)
    pub(crate) hits: [Vec<Hit>; 2],
    /// the drop targets of the same two passes (dnd.rs)
    pub(crate) zones: [Vec<super::dnd::Zone>; 2],
    pub(crate) keyq: Vec<u32>,
    pub(crate) textq: Vec<u32>,
    pub(crate) hit_cur: usize,
    pub(crate) mx: i32,
    pub(crate) my: i32,
    pub(crate) mbtn: u32,
    pub(crate) pressed: u32,
    pub(crate) released: u32,
    pub(crate) wheel_x: i32,
    pub(crate) wheel_y: i32,
    pub(crate) moved: bool,
    pub(crate) press_x: i32,
    pub(crate) press_y: i32,
    pub(crate) hot: u32,
    pub(crate) hot_flags: u32,
    pub(crate) active: u32,
    pub(crate) active_seen: bool,
    pub(crate) ractive: u32,
    pub(crate) focus: u32,
    pub(crate) focus_visible: bool,
    pub(crate) press_used: bool,
    pub(crate) rpress_used: bool,
    pub(crate) last_click_id: u32,
    pub(crate) last_click_ms: u32,
    pub(crate) last_state: u32,
    pub(crate) modal_cur: u32,
    pub(crate) modal_prev: u32,
    pub(crate) disabled_depth: u32,
    pub(crate) cmd_layer_ui: u32,
    /// keyboard navigation of this window (kbnav.rs)
    pub(crate) kb: super::kbnav::W,
}

impl W {
    /// a window before its first pass (pointer outside: -1, -1)
    pub(crate) fn new() -> W {
        W {
            last_rect: [0; 4],
            uiclip: [0; 4],
            uiclip_stack: Vec::new(),
            uiclip_over: 0,
            hits: [Vec::new(), Vec::new()],
            zones: [Vec::new(), Vec::new()],
            keyq: Vec::new(),
            textq: Vec::new(),
            hit_cur: 0,
            mx: -1,
            my: -1,
            mbtn: 0,
            pressed: 0,
            released: 0,
            wheel_x: 0,
            wheel_y: 0,
            moved: false,
            press_x: 0,
            press_y: 0,
            hot: 0,
            hot_flags: 0,
            active: 0,
            active_seen: false,
            ractive: 0,
            focus: 0,
            focus_visible: false,
            press_used: false,
            rpress_used: false,
            last_click_id: 0,
            last_click_ms: 0,
            last_state: 0,
            modal_cur: 0,
            modal_prev: 0,
            disabled_depth: 0,
            cmd_layer_ui: 0,
            kb: Default::default(),
        }
    }
}

/// hit_find_list(list, modal_layer, x, y) -> (id, flags) of the topmost hit box at the
/// point: layers from the top down to modal_layer, last record first; HB_BLOCK /
/// HB_DISABLED boxes answer id 0.  (-1, -1) = outside the window: none.
pub(crate) fn hit_find_list(list: &[Hit], modal_layer: u32, x: i32, y: i32) -> (u32, u32) {
    // outside the window = (-1, -1) (E_LEAVE); other negative points are real: popups of a
    // secondary window may lie left of / above it
    if x == -1 && y == -1 {
        return (0, 0);
    }
    let mut layer = L_COUNT as i32 - 1;
    while layer >= modal_layer as i32 {
        for h in list.iter().rev() {
            if h.layer as i32 != layer || x < h.x0 || x >= h.x1 || y < h.y0 || y >= h.y1 {
                continue;
            }
            let flags = h.flags as u32;
            let id = if flags & (HB_BLOCK | HB_DISABLED) != 0 { 0 } else { h.id };
            return (id, flags);
        }
        layer -= 1;
    }
    (0, 0)
}

impl Ui {
    // ------------------------------------------------------------------ small queries

    /// ui_now: milliseconds since start (virtual clock in headless runs)
    pub fn now(&self) -> u32 {
        self.now_ms
    }

    /// ui_mods: MOD_* bits of the current input
    pub fn mods(&self) -> u32 {
        self.mods
    }

    /// ui_mouse: pointer position in px (-1 when outside)
    pub fn mouse(&self) -> (i32, i32) {
        (self.wp.ctx.mx, self.wp.ctx.my)
    }

    /// ui_pen: (pressure 0 = none / 1..UI_PRESSURE_ONE, PTR_*) of the last pointer event
    pub fn pen(&self) -> (u32, u32) {
        (self.mpress, self.mptype)
    }

    /// ui_focus(id): give the keyboard focus to a widget (focus ring shown).  The id is
    /// looked up in whichever window emits it in the next pass (that window is activated).
    pub fn focus(&mut self, id: u32) {
        self.focus_req_old = self.wp.ctx.focus;
        self.focus_req = id;
        self.focus_req_win = self.uiw_cur;
        self.focus_req_frame = self.frame;
        self.wp.ctx.focus = id;
        self.wp.ctx.focus_visible = true;
        self.repass = true;
    }

    /// ui_focus_req_expire: after the passes, a focus id no window emitted is dropped
    pub(crate) fn focus_req_expire(&mut self) {
        if self.focus_req_frame < self.frame {
            self.focus_req = 0;
        }
    }

    /// ui_focused: id of the focused widget (0 = none)
    pub fn focused(&self) -> u32 {
        self.wp.ctx.focus
    }

    // ui_key_find: index of an unconsumed queue entry for vk
    fn key_find(&self, vk: u32) -> Option<usize> {
        self.wp.ctx.keyq.iter().position(|&k| (k as i32) >= 0 && k & 0xFFFF == vk)
    }

    /// ui_key(vk): pressed this frame and not yet consumed (consumes it)
    pub fn key(&mut self, vk: u32) -> bool {
        let k = self.key_find(vk);
        match self.text_key_filter(k) {
            Some(i) => {
                self.wp.ctx.keyq[i] |= 0x8000_0000;
                true
            }
            None => false,
        }
    }

    /// Capture an exact application shortcut before text widgets handle it. Unlike key(),
    /// this bypasses the focused textbox's key reservation. Use only for an editing
    /// surface whose history/commands the application owns, and not while a modal draft
    /// dialog owns input. Other shortcuts and the normal key() behavior are unchanged.
    pub fn capture_shortcut(&mut self, vk: u32, mods: u32) -> bool {
        if let Some(i) = self.wp.ctx.keyq.iter().position(|&k| {
            (k as i32) >= 0 && k & 0xffff == vk && (k >> 16) == mods
        }) {
            self.wp.ctx.keyq[i] |= 0x8000_0000;
            true
        } else {
            false
        }
    }

    /// ui_key_peek(vk): pressed this frame and unconsumed (not consumed)
    pub fn key_peek(&mut self, vk: u32) -> bool {
        let k = self.key_find(vk);
        self.text_key_filter(k).is_some()
    }

    /// ui_key_down(vk): held (mods: VK_SHIFT / VK_CONTROL / VK_MENU)
    pub fn key_down(&self, vk: u32) -> bool {
        self.keydown[(vk & 255) as usize] != 0
    }

    /// ui_key_first: the first unconsumed key this frame (VK | mods << 16) or 0
    pub fn key_first(&self) -> u32 {
        self.wp.ctx.keyq.iter().copied().find(|&k| (k as i32) >= 0).unwrap_or(0)
    }

    /// ui_text_take(cap): the typed codepoints (at most cap), consumed; when cap leaves some,
    /// they come in the next pass (nothing typed is lost)
    pub fn text_take(&mut self, cap: usize) -> Vec<u32> {
        let c = &mut self.wp.ctx;
        let n = c.textq.len().min(cap);
        let v: Vec<u32> = c.textq.drain(..n).collect();
        if n > 0 && !c.textq.is_empty() {
            let rest = core::mem::take(&mut c.textq);
            self.text_defer(rest);
        }
        self.wp.ctx.textq.clear(); // (cap 0 takes nothing: the text goes)
        v
    }

    // text_defer(rest): typed text of the live window for the next pass, before the text
    // that waits already (it came later)
    fn text_defer(&mut self, rest: Vec<u32>) {
        let w = self.uiw_cur;
        match self.text_more.first_mut() {
            Some((m, q)) if *m == w => drop(q.splice(0..0, rest)),
            _ => self.text_more.insert(0, (w, rest)),
        }
    }

    // text_push(cp): a typed codepoint for the live window; beyond TEXTQ_MAX (or behind text
    // that waits already) it waits for the next pass (text_refill)
    fn text_push(&mut self, cp: u32) {
        if self.text_more.is_empty() && self.wp.ctx.textq.len() < TEXTQ_MAX {
            self.wp.ctx.textq.push(cp);
            return;
        }
        let w = self.uiw_cur;
        match self.text_more.last_mut() {
            Some((m, q)) if *m == w => q.push(cp),
            _ => self.text_more.push((w, alloc::vec![cp])),
        }
    }

    /// text_refill -> true: the next part of the text that waits (text_more) is in its
    /// window's queue for the next passes (run_passes); text of a window that is gone goes
    pub(crate) fn text_refill(&mut self) -> bool {
        while let Some((w, q)) = self.text_more.first() {
            let w = *w;
            if q.is_empty() || (w != 0 && !self.uiw_drawable(w)) {
                self.text_more.remove(0);
                continue;
            }
            let cur = self.uiw_cur;
            self.uiw_use(w);
            let q = &mut self.text_more[0].1;
            let n = q.len().min(TEXTQ_MAX.saturating_sub(self.wp.ctx.textq.len()));
            self.wp.ctx.textq.extend(q.drain(..n));
            if q.is_empty() {
                self.text_more.remove(0);
            }
            self.m.win.edges |= 1 << (w & (WIN_MAX as u32 - 1)); // cleared after the first pass
            self.uiw_fwd_arm(w);
            self.uiw_use(cur);
            return true;
        }
        false
    }

    /// ui_keys_take_rest(cap): (U3, key forwarding of tool windows) the unconsumed keys of
    /// this frame (VK | mods << 16), consumed
    #[allow(dead_code)] // ui-win: key forwarding of tool windows (U3)
    pub(crate) fn keys_take_rest(&mut self, cap: usize) -> Vec<u32> {
        let mut out = Vec::new();
        for k in self.wp.ctx.keyq.iter_mut() {
            if (*k as i32) < 0 {
                continue;
            }
            if out.len() >= cap {
                break;
            }
            out.push(*k);
            *k |= 0x8000_0000;
        }
        out
    }

    /// ui_keys_inject(keys): (U3) append keys to the live window's queue (the forwarded keys
    /// of a tool window; the script command `keys`)
    pub(crate) fn keys_inject(&mut self, keys: &[u32]) {
        for &k in keys {
            self.key_push(k & 0x7FFF_FFFF);
        }
    }

    // key_push(k): a key (VK | mods << 16) for the live window's pass
    fn key_push(&mut self, k: u32) {
        if self.wp.ctx.keyq.len() < KEYQ_MAX {
            self.wp.ctx.keyq.push(k);
        } else {
            self.limit_hit(LIM_KEYS);
        }
    }

    /// limit_hit(lim): a ceiling (LIM_*) held something back: counted in Ui::limits, a
    /// UIWARN the first time
    pub(crate) fn limit_hit(&mut self, lim: usize) {
        let n = self.limits[lim];
        self.limits[lim] = n.saturating_add(1);
        if n == 0 {
            self.warn(LIM_TEXT[lim]);
        }
    }

    /// ui_text_inject(cps): (U3) append typed codepoints to the live window's text queue
    #[allow(dead_code)] // ui-win: key forwarding of tool windows (U3)
    pub(crate) fn text_inject(&mut self, cps: &[u32]) {
        // text of this pass: before any that waits; what does not fit, in the next pass
        let n = TEXTQ_MAX.saturating_sub(self.wp.ctx.textq.len()).min(cps.len());
        self.wp.ctx.textq.extend_from_slice(&cps[..n]);
        if n < cps.len() {
            self.text_defer(cps[n..].to_vec());
        }
    }

    // ------------------------------------------------------------------ input feed

    /// ui_input_event(ev) -> 0 nothing, 1 an edge (its own pass), 2 text (batched)
    pub(crate) fn input_event(&mut self, ev: &Event) -> u32 {
        let mut e = *ev;
        self.rtl_input(&mut e); // a mirrored window: logical pointer and arrows
        let ev = &e;
        self.mods = ev.mods as u32;
        match ev.typ {
            plat::E_MOVE => {
                self.wp.ctx.mx = ev.x;
                self.wp.ctx.my = ev.y;
                self.pen_decode(ev.data);
                self.pts_add();
                self.wp.ctx.moved = true;
                self.dnd_moved(); // a drag that leaves the window goes to the platform
                0
            }
            plat::E_DOWN => {
                if ev.x != -1 {
                    self.wp.ctx.mx = ev.x;
                    self.wp.ctx.my = ev.y;
                }
                self.pen_decode(ev.data);
                let b = (ev.btn & 31) as u32;
                let c = &mut self.wp.ctx;
                c.mbtn |= 1 << b;
                c.pressed |= 1 << b;
                c.press_x = c.mx;
                c.press_y = c.my;
                self.mpts.clear();
                self.mprs.clear();
                self.pts_add();
                1
            }
            plat::E_UP => {
                self.dnd_input(ev); // the release of a drag: the drop
                let c = &mut self.wp.ctx;
                if ev.x != -1 {
                    c.mx = ev.x;
                    c.my = ev.y;
                }
                let b = (ev.btn & 31) as u32;
                c.mbtn &= !(1 << b);
                c.released |= 1 << b;
                1
            }
            plat::E_WHEEL => {
                let c = &mut self.wp.ctx;
                c.wheel_y = c.wheel_y.wrapping_add(ev.data as u16 as i16 as i32);
                c.wheel_x = c.wheel_x.wrapping_add((ev.data as i32) >> 16);
                1
            }
            plat::E_KEY_DOWN => {
                if self.dnd_input(ev) {
                    return 1; // Esc ended a drag
                }
                let vk = ev.data & 0xFFFF;
                self.keydown[(vk & 255) as usize] = 1;
                self.key_push((vk | self.mods << 16) & 0x7FFF_FFFF);
                1
            }
            plat::E_KEY_UP => {
                let k = ev.data & 255;
                self.keydown[k as usize] = 0;
                // (popup agent) Alt release: a pass for the menu bar's Alt tap
                (k == VK_MENU) as u32
            }
            plat::E_TEXT => {
                self.text_push(ev.data);
                2 // text batches with following text
            }
            plat::E_PREEDIT => 1,
            plat::E_IME_COMMIT => {
                if let Some(text) = plat::ime_commit_take(ev.data) {
                    self.hr_commit(&text); // (hotreload.rs) the commit's text into its record
                    for c in text.chars() {
                        self.text_push(c as u32);
                    }
                }
                1 // atomic input, handled in its own pass
            }
            plat::E_LEAVE => {
                let c = &mut self.wp.ctx;
                c.mx = -1;
                c.my = -1;
                c.moved = true;
                0
            }
            plat::E_FOCUS_OUT => {
                self.wp.ctx.mbtn = 0;
                self.keydown = [0; 256];
                1
            }
            _ => 0,
        }
    }

    // pen_decode(data): (PN1) Event::data of a pointer event -> pressure, type, tilt
    fn pen_decode(&mut self, d: u32) {
        self.mpress = d & plat::PD_PRESSURE;
        self.mptype = (d >> plat::PD_TYPE_SHIFT) & 3;
        self.mtilt[0] = ((d << (32 - plat::PD_TILTY_SHIFT)) as i32) >> 25;
        self.mtilt[1] = (d as i32) >> plat::PD_TILTY_SHIFT;
    }

    // pts_add: remember the pointer position for stroke-like widgets (one window's points)
    fn pts_add(&mut self) {
        if self.uiw_cur != self.mpts_win {
            self.mpts_win = self.uiw_cur;
            self.mpts.clear();
            self.mprs.clear();
        }
        if self.mpts.len() < PTS_MAX {
            self.mpts.push([self.wp.ctx.mx, self.wp.ctx.my]);
            self.mprs.push(self.mpress);
        } else {
            self.limit_hit(LIM_POINTS);
        }
    }

    /// ui_input_clear_edges: after the first pass of a batch the edges are consumed
    pub(crate) fn input_clear_edges(&mut self) {
        let c = &mut self.wp.ctx;
        c.pressed = 0;
        c.released = 0;
        c.wheel_x = 0;
        c.wheel_y = 0;
        c.keyq.clear();
        c.textq.clear();
        c.moved = false;
        // keep the last pointer position as the start of the next stroke segment
        if self.uiw_cur == self.mpts_win && !self.mpts.is_empty() {
            let n = self.mpts.len();
            let (p, r) = (self.mpts[n - 1], self.mprs[n - 1]);
            self.mpts.clear();
            self.mprs.clear();
            self.mpts.push(p);
            self.mprs.push(r);
        }
    }

    /// ui_set_time(ms)
    pub(crate) fn set_time(&mut self, ms: u32) {
        self.now_ms = ms;
        if self.headless {
            plat::task::clock_set(Some(ms as u64 * 1000)); // task timers follow the virtual clock
        }
    }

    // ------------------------------------------------------------------ hit testing

    /// ui_hot_find(x, y) -> (id, flags) at the point in the previous pass (0 in a window
    /// made inert by a modal window)
    pub(crate) fn hot_find(&self, x: i32, y: i32) -> (u32, u32) {
        if self.uiw_inert_now() {
            return (0, 0);
        }
        let c = &self.wp.ctx;
        hit_find_list(&c.hits[c.hit_cur ^ 1], c.modal_prev, x, y)
    }

    /// ui_hit_flags_at(x, y): HB_* flags of the topmost hit box at a point of the main
    /// window (previous pass) -- non-client hit testing of the platform
    pub fn hit_flags_at(&mut self, x: i32, y: i32) -> u32 {
        self.hit_flags_win(0, x, y)
    }

    /// ui_hot_update -> true if the hot widget changed (sets the cursor too)
    pub(crate) fn hot_update(&mut self) -> bool {
        let (id, flags) = self.hot_find(self.wp.ctx.mx, self.wp.ctx.my);
        // cursor: the active widget keeps its cursor
        if self.wp.ctx.active == 0 {
            let mut shape = (flags >> HB_CURSOR_SHIFT) & 15;
            if flags & HB_DRAG != 0 {
                shape = 0;
            }
            let s = self.cursor_resolve(shape);
            plat::set_cursor(s);
        }
        let c = &mut self.wp.ctx;
        let changed = id != c.hot;
        c.hot = id;
        c.hot_flags = flags;
        changed
    }

    /// ui_needs_move_pass: a pointer move must be answered with a pass
    pub(crate) fn needs_move_pass(&mut self) -> bool {
        self.hot_update() || self.wp.ctx.active != 0 || self.wp.ctx.hot_flags & HB_MOVES != 0 || self.dnd_follows_pointer()
    }

    /// ui_hit(id, hbflags): register last_rect (clipped) as a hit box of the current layer
    pub fn hit(&mut self, id: u32, hbflags: u32) {
        let c = &mut self.wp.ctx;
        let (group, pane, notab) = c.kb.hit_keys(hbflags);
        if c.hits[c.hit_cur].len() >= HITS_MAX {
            self.limit_hit(LIM_HITS);
            return;
        }
        let c = &mut self.wp.ctx;
        let list = &mut c.hits[c.hit_cur];
        let r = c.last_rect;
        let k = c.uiclip;
        list.push(Hit {
            id,
            continuation: false,
            flags: (hbflags | notab) as u16,
            layer: c.cmd_layer_ui as u8,
            kind: 0,
            group,
            pane,
            x0: r[0].max(k[0]),
            y0: r[1].max(k[1]),
            x1: r[0].wrapping_add(r[2]).min(k[2]),
            y1: r[1].wrapping_add(r[3]).min(k[3]),
        });
    }

    /// ui_behave(id, hbflags) -> EV_*: the standard pointer / keyboard behaviour of a
    /// widget occupying last_rect
    pub fn behave(&mut self, id: u32, hbflags: u32) -> u32 {
        // a pending focus(id) for this widget (it may live in another window)
        if self.focus_req == id && id != 0 {
            self.uiw_focus_arrived();
        }
        let mut flags = hbflags;
        // inside a disabled box everything is disabled
        if self.wp.ctx.disabled_depth != 0 {
            flags |= HB_DISABLED;
        }
        self.hit(id, flags);
        self.wp.ctx.last_state = 0;
        if flags & HB_DISABLED != 0 {
            self.wp.ctx.last_state = SS_DISABLED as u32;
            return 0;
        }
        let now = self.now_ms;
        let lim = self.dp(4);
        let dbl_ms = self.dbl_ms; // the system's double-click time
        let mut ev = 0;
        let c = &mut self.wp.ctx;
        if c.hot == id {
            ev |= EV_HOVER;
            if c.pressed & (1 << BTN_LEFT) != 0 && !c.press_used {
                c.press_used = true;
                c.active = id;
                if flags & HB_FOCUS != 0 {
                    c.focus = id;
                    c.focus_visible = false;
                }
                if c.last_click_id == id && now.wrapping_sub(c.last_click_ms) <= dbl_ms {
                    ev |= EV_DOUBLE;
                    c.last_click_id = 0;
                } else {
                    c.last_click_id = id;
                    c.last_click_ms = now;
                }
            }
            if c.pressed & (1 << BTN_RIGHT) != 0 && !c.rpress_used {
                c.rpress_used = true;
                c.ractive = id;
            }
        }
        // active (held) widget
        if c.active == id {
            c.active_seen = true;
            if c.mbtn & (1 << BTN_LEFT) != 0 {
                ev |= EV_HELD;
                // dragged more than 4 DIP?
                let dx = c.mx.wrapping_sub(c.press_x).wrapping_abs();
                let dy = c.my.wrapping_sub(c.press_y).wrapping_abs();
                if dx > lim || dy > lim {
                    ev |= EV_DRAG;
                }
            } else {
                c.active = 0;
                ev |= EV_COMMIT;
                if ev & EV_HOVER != 0 {
                    ev |= EV_ACT;
                }
            }
        }
        if c.ractive == id && c.mbtn & (1 << BTN_RIGHT) == 0 {
            c.ractive = 0;
            if ev & EV_HOVER != 0 {
                ev |= EV_RCLICK;
            }
        }
        if c.focus == id {
            ev |= EV_FOCUS;
            if flags & (HB_TEXT | HB_OWNKEYS) == 0 && flags & HB_FOCUS != 0 && (self.key(VK_SPACE) || flags & HB_SPACE == 0 && self.key(VK_RETURN)) {
                ev |= EV_ACT;
                self.wp.ctx.focus_visible = true;
            }
        }
        // the menu key / Shift+F10 that no widget took: a right click of the focused widget
        // (kbnav.rs)
        if self.m.kbnav.ctx == id && id != 0 {
            ev |= self.kb_ctx_take();
        }
        // a click of a screen reader (a11y.rs) acts like Space
        if self.m.a11y.pending() && flags & HB_TEXT == 0 && self.a11y_click(id) {
            ev |= EV_ACT;
        }
        // an action usually changes app state that was already drawn: build the frame
        // again so everything shows the new state
        if ev & (EV_ACT | EV_COMMIT | EV_DOUBLE | EV_RCLICK) != 0 {
            self.repass = true;
        }
        // semantic state for the dump
        let mut st = 0;
        if ev & EV_HOVER != 0 {
            st |= SS_HOVER;
        }
        if ev & EV_HELD != 0 {
            st |= SS_PRESSED;
        }
        if ev & EV_FOCUS != 0 {
            st |= SS_FOCUSED;
        }
        self.wp.ctx.last_state = st as u32;
        ev
    }

    /// ui_hit_rect(id, x, y, w, h, hbflags) -> EV_*: behave on an explicit rectangle (px)
    pub fn hit_rect(&mut self, id: u32, x: i32, y: i32, w: i32, h: i32, hbflags: u32) -> u32 {
        self.wp.ctx.last_rect = [x, y, w, h];
        self.behave(id, hbflags)
    }

    /// ui_focus_ring(ev): the focus visual should be drawn for the widget that just
    /// called behave (its events)
    pub fn focus_ring(&self, ev: u32) -> bool {
        ev & EV_FOCUS != 0 && self.wp.ctx.focus_visible
    }

    // ------------------------------------------------------------------ state slots

    /// ui_slot(id): persistent per-id state (created on first use)
    pub fn slot(&mut self, id: u32) -> &mut Slot {
        let frame = self.frame;
        let s = self.slots.entry(id).or_default();
        s.seen = frame;
        s
    }

    /// ui_anim(id, target, ms) -> current value (0..65535, usually 0..256) moving towards
    /// target (ease-out)
    pub fn anim(&mut self, id: u32, target: u32, ms: u32) -> u32 {
        self.anim_ex(id, target, ms, EASE_OUT)
    }

    /// anim_ex(id, target, ms, curve) -> current value moving towards target along an
    /// EASE_* curve (EASE_OUT_BACK may overshoot; the value stays >= 0)
    pub fn anim_ex(&mut self, id: u32, target: u32, ms: u32, curve: u32) -> u32 {
        let now = self.now_ms;
        let off = self.anim_off;
        // animation slots live beside widget slots; aux keeps the curve
        let s = self.slot(id | 0x4000_0000);
        s.aux = curve as i32;
        // value: interpolated along the curve
        fn value(s: &Slot, now: u32) -> u32 {
            let to = s.ato as u32;
            let dur = s.adur;
            if dur == 0 {
                return to;
            }
            let el = now.wrapping_sub(s.at0);
            if el >= dur {
                return to;
            }
            let t = ((el << 8) / dur) as i32;
            let e = ease(s.aux as u32, t);
            let from = s.afrom as i32;
            let d = ((to as i32 - from).wrapping_mul(e)) >> 8;
            (from + d).max(0) as u32
        }
        if off || s.adur == 0 {
            // first use: start at the target
            s.afrom = target as u16;
            s.ato = target as u16;
            s.adur = if ms == 0 { 1 } else { ms };
            return target;
        }
        if s.ato as u32 != target {
            // new target: start from the current value
            let v = value(s, now);
            s.afrom = v as u16;
            s.ato = target as u16;
            s.at0 = now;
            s.adur = ms;
        }
        let v = value(s, now);
        if v != s.ato as u32 {
            self.animating = true;
        }
        v
    }

    /// anim_color(id, argb, ms, curve) -> the colour on its way to argb (a transition: hover
    /// fills, state colours); the first call starts at argb
    pub fn anim_color(&mut self, id: u32, argb: u32, ms: u32, curve: u32) -> u32 {
        let now = self.now_ms;
        let off = self.anim_off;
        // colour slots: measw = from, meash = to, at0 / adur the time, aux the curve
        let s = self.slot(id | 0x2000_0000);
        let at = |s: &Slot| -> u32 {
            let el = now.wrapping_sub(s.at0);
            if s.adur == 0 || el >= s.adur {
                return s.meash as u32;
            }
            let t = ((el << 8) / s.adur) as i32;
            super::theme::color_mix(s.measw as u32, s.meash as u32, ease(s.aux as u32, t).clamp(0, 256))
        };
        if off || s.adur == 0 {
            s.measw = argb as i32;
            s.meash = argb as i32;
            s.adur = ms.max(1);
            s.at0 = now;
            return argb;
        }
        if s.meash as u32 != argb {
            let c = at(s);
            s.measw = c as i32;
            s.meash = argb as i32;
            s.at0 = now;
            s.adur = ms.max(1);
            s.aux = curve as i32;
        }
        let c = at(s);
        if c != argb {
            self.animating = true;
        }
        c
    }

    // ------------------------------------------------------------------ commands, timers

    /// ui_cmd_run(id, arg): queue a command for App::command after this frame (in order)
    pub fn cmd_run(&mut self, id: u32, arg: u32) {
        if self.cmdq.len() < CMDQ_MAX {
            self.cmdq.push_back((id, arg));
        } else {
            self.limit_hit(LIM_CMDS);
        }
    }

    /// ui_cmd_take -> the oldest queued command
    pub(crate) fn cmd_take(&mut self) -> Option<(u32, u32)> {
        self.cmdq.pop_front()
    }

    /// ui_timer(id, ms): App::command(UIC_TIMER, id) after ms (replaces a timer with the
    /// same id)
    pub fn timer(&mut self, id: u32, ms: u32) {
        let deadline = self.now_ms.wrapping_add(ms);
        // set again: behind the others (of equal deadlines the one set first fires first)
        if let Some(i) = self.timers.iter().position(|t| t.0 == id) {
            self.timers.remove(i);
        } else if self.timers.len() >= TIMERS_MAX {
            self.limit_hit(LIM_TIMERS);
            return;
        }
        self.timers.push((id, deadline));
    }

    /// ui_timer_due -> (id of the expired timer with the earliest deadline (removed; of
    /// equal deadlines the one set first) or -1, the next deadline ms or -1)
    pub(crate) fn timer_due(&mut self) -> (u32, u32) {
        match self.timer_scan() {
            (Some(i), next) => (self.timers.remove(i).0, next),
            (None, next) => (u32::MAX, next),
        }
    }

    /// timer_scan -> (index of the expired timer timer_due takes next, the earliest deadline
    /// still ahead or -1)
    pub(crate) fn timer_scan(&self) -> (Option<usize>, u32) {
        let now = self.now_ms;
        let (mut due, mut due_at) = (None, 0i32);
        let (mut next, mut left) = (u32::MAX, u32::MAX);
        for (i, &(_, d)) in self.timers.iter().enumerate() {
            let rel = d.wrapping_sub(now) as i32; // ms until the deadline (wraps with the clock)
            if rel <= 0 {
                if due.is_none() || rel < due_at {
                    due = Some(i);
                    due_at = rel;
                }
            } else if (rel as u32) < left {
                left = rel as u32;
                next = d;
            }
        }
        (due, next)
    }

    // ------------------------------------------------------------------ semantic records

    /// g_last_rect = 0, g_last_state = 0: the apps' hidden probe records (PaintCraft's *_sem
    /// helpers zero both before ui_sem; later records of the pass see the zeroes too)  // pn-core
    pub fn last_reset(&mut self) {
        self.wp.ctx.last_rect = [0; 4];
        self.wp.ctx.last_state = 0;
    }

    /// g_last_rect = rect, g_last_state = state (apps that build a widget from drawing calls
    /// set the record's rectangle and state before ui_sem)  // pn-core
    pub fn last_set(&mut self, rect: [i32; 4], state: u32) {
        self.wp.ctx.last_rect = rect;
        self.wp.ctx.last_state = state;
    }

    /// g_last_state: the SS_* state of the last behave  // pn-core
    pub fn last_state(&self) -> u32 {
        self.wp.ctx.last_state
    }

    /// g_mbtn / g_pressed / g_released: mouse buttons held / pressed / released this frame
    /// (bit 1 << BTN_*)  // pn-core
    pub fn mbtn(&self) -> u32 {
        self.wp.ctx.mbtn
    }
    pub fn pressed(&self) -> u32 {
        self.wp.ctx.pressed
    }
    pub fn released(&self) -> u32 {
        self.wp.ctx.released
    }

    /// ui_sem(kind, id, label, value) -> the record (rect = last_rect, state from the last
    /// behave); the label is copied
    pub fn sem(&mut self, kind: u16, id: u32, label: Option<&str>, value: i32) -> &mut Sem {
        if self.sems.len() >= SEMS_MAX {
            self.limit_hit(LIM_SEMS);
            return &mut self.sem_dummy;
        }
        let c = &self.wp.ctx;
        let mut r = Sem {
            id,
            kind,
            state: c.last_state as u16,
            rect: self.phys_rect(c.last_rect), // window px (mirrored right-to-left)
            label: None,
            value,
            max: 0,
            min: 0,
            layer: c.cmd_layer_ui as u8,
            depth: self.wp.lay.boxes.len() as u8,
            win: self.uiw_cur as u8,
        };
        if let Some(l) = label {
            // copy the label into the frame text arena (it may live in a scratch buffer)
            let l = l.split('\0').next().unwrap_or("");
            let n = self.semtext_len + l.len() + 1;
            if n <= SEMTEXT_MAX {
                self.semtext_len = n;
                r.label = Some(String::from(l));
            } else {
                self.limit_hit(LIM_SEMTEXT);
            }
        }
        self.sems.push(r);
        let n = self.sems.len();
        &mut self.sems[n - 1]
    }

    // ------------------------------------------------------------------ clipping, layers

    /// ui_clip_push(x, y, w, h): px; intersects drawing and hit clipping
    pub fn clip_push(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.draw_clip(x, y, w, h);
        let c = &mut self.wp.ctx;
        if c.uiclip_stack.len() >= UICLIP_MAX {
            c.uiclip_over += 1;
            self.limit_hit(LIM_CLIP);
            return;
        }
        c.uiclip_stack.push(c.uiclip);
        let k = &mut c.uiclip;
        k[0] = x.max(k[0]);
        k[1] = y.max(k[1]);
        k[2] = x.wrapping_add(w).min(k[2]);
        k[3] = y.wrapping_add(h).min(k[3]);
    }

    /// ui_clip_pop
    pub fn clip_pop(&mut self) {
        self.draw_unclip();
        let c = &mut self.wp.ctx;
        if c.uiclip_over != 0 {
            c.uiclip_over -= 1; // a level beyond UICLIP_MAX: it clipped nothing
        } else if let Some(k) = c.uiclip_stack.pop() {
            c.uiclip = k;
        }
    }

    /// ui_layer_set(layer) -> previous: switch the command layer and hit layer together
    pub fn layer_set(&mut self, layer: u32) -> u32 {
        let prev = self.wp.ctx.cmd_layer_ui;
        self.wp.ctx.cmd_layer_ui = layer & 3;
        self.cmd_layer(layer);
        prev
    }

    // ------------------------------------------------------------------ passes
    //
    // A pass builds every window: frame_begin (shared state), then per window
    // uiw_pass_begin .. content .. uiw_pass_end (window 0 by the runtime, the others inside
    // window_begin / window_end), then frame_end.

    /// ui_begin_pass(w, h): start recording a frame for a w x h px window (one window)
    pub fn begin_pass(&mut self, w: i32, h: i32) {
        self.frame_begin();
        self.uiw_pass_begin(w, h);
    }

    /// ui_end_pass -> another pass is needed (one window)
    pub fn end_pass(&mut self) -> bool {
        self.uiw_pass_end();
        self.frame_end()
    }

    /// ui_frame_begin: start of a pass over all windows
    pub(crate) fn frame_begin(&mut self) {
        self.repass = false;
        self.sems.clear();
        self.semtext_len = 0;
        self.animating = false;
        self.frame = self.frame.wrapping_add(1);
        if self.m.ins.on {
            self.m.ins.boxes.clear(); // the inspector's boxes of this pass (inspect.rs)
        }
        self.rtl_update();
        self.ids_frame_begin();
        self.fonts_frame(); // the fallback follows the UI language
        self.uiw_frame_begin();
    }

    /// ui_frame_end -> another pass is needed before the frame is final
    pub(crate) fn frame_end(&mut self) -> bool {
        self.uiw_frame_end();
        self.repass
    }

    /// uiw_pass_begin(w, h): start recording the live window (w x h px): empty command
    /// lists and hit list, background, root layout box
    pub(crate) fn uiw_pass_begin(&mut self, w: i32, h: i32) {
        self.cmd_reset();
        let c = &mut self.wp.ctx;
        c.cmd_layer_ui = 0;
        c.press_used = false;
        c.rpress_used = false;
        c.active_seen = false;
        c.modal_cur = 0;
        c.disabled_depth = 0;
        c.kb.pass_begin();
        c.uiclip_stack.clear();
        c.uiclip_over = 0;
        c.uiclip = [0, 0, w, h];
        let cur = c.hit_cur;
        c.hits[cur].clear();
        c.zones[cur].clear();
        self.hot_update();
        self.dnd_pass_begin(); // the drop target under a drag (previous pass)
        // window background (style)
        let st = self.style;
        st.window_bg(self, w, h);
        // root layout box (secondary windows: inside their 1 px border)
        let b = self.uiw_border();
        self.layout_root(b, b, w - b - b, h - b - b);
    }

    /// uiw_pass_end: finish the live window's pass: runtime menus and accelerators, layout,
    /// Tab, Esc, modal layer, hit lists
    pub(crate) fn uiw_pass_end(&mut self) {
        self.uiw_fwd_take(); // (U3) keys a tool window borrowed and left
        self.accel_dispatch(); // (popup agent) runtime menus, stale popups, accelerators
        self.layout_end_root();
        // an active widget that vanished releases the pointer
        if !self.wp.ctx.active_seen {
            self.wp.ctx.active = 0;
        }
        // Tab / Shift+Tab, focus groups (arrows), F6 / Ctrl+F6 (panes, windows), F8
        // (splitters), the menu key (kbnav.rs)
        self.kb_nav();
        // Esc not consumed by a widget: UIC_ESCAPE (dialog windows: a close request)
        if self.key(VK_ESCAPE) {
            self.uiw_escape();
        }
        let c = &mut self.wp.ctx;
        if c.modal_cur != c.modal_prev {
            c.modal_prev = c.modal_cur;
            self.repass = true;
        }
        if self.dup_check {
            self.dup_ids();
        }
        self.dnd_pass_end(); // the label next to the pointer of a drag
        // swap hit lists
        self.wp.ctx.hit_cur ^= 1;
        self.uiw_publish_hits(); // (the platform's hit hook; see win.rs)
    }

    // dup_ids: an id that two widgets of this pass share (they would share hover, press
    // and focus: a click on one acts on the other) -- one UIWARN per id
    fn dup_ids(&mut self) {
        let c = &self.wp.ctx;
        // (a widget registering the same rectangle twice is still one widget)
        let mut ids: Vec<(u32, i32, i32, i32, i32)> =
            c.hits[c.hit_cur].iter().filter(|h| h.id != 0 && !h.continuation).map(|h| (h.id, h.x0, h.y0, h.x1, h.y1)).collect();
        ids.sort_unstable();
        ids.dedup();
        let dups: Vec<((u32, i32, i32, i32, i32), (u32, i32, i32, i32, i32))> = ids.windows(2).filter(|w| w[0].0 == w[1].0).map(|w| (w[0], w[1])).collect();
        for (a, b) in dups {
            if self.dup_warned.insert(a.0) {
                let name = self.id_name(a.0).map_or(String::new(), |n| alloc::format!(" (\"{}\")", n));
                self.warn(&alloc::format!("widget id {:#X}{} used twice in one pass (at {},{} and {},{})", a.0, name, a.1, a.2, b.1, b.2));
            }
        }
    }

    /// ui_repass: request another pass before this frame is shown
    pub fn repass(&mut self) {
        self.repass = true;
    }

    /// ui_modal(layer): a modal element (dialog) is open on `layer` this pass
    pub fn modal(&mut self, layer: u32) {
        if layer as i32 > self.wp.ctx.modal_cur as i32 {
            self.wp.ctx.modal_cur = layer;
        }
    }

    /// ui_modal_active -> layer of the modal element of the last pass (0 = none)
    pub fn modal_active(&self) -> u32 {
        self.wp.ctx.modal_prev
    }

    /// ui_disabled_push / ui_disabled_pop: widgets in between are disabled
    pub fn disabled_push(&mut self) {
        self.wp.ctx.disabled_depth += 1;
    }

    pub fn disabled_pop(&mut self) {
        if self.wp.ctx.disabled_depth != 0 {
            self.wp.ctx.disabled_depth -= 1;
        }
    }

    /// ui_warn: "UIWARN f<frame>: <msg>" on stderr
    pub fn warn(&mut self, msg: &str) {
        self.warns = self.warns.wrapping_add(1);
        let s = alloc::format!("UIWARN f{}: {}\n", self.frame, msg);
        plat::write_stderr(s.as_bytes());
        plat::log_quiet(plat::LOG_WARN, "ui", msg); // the ring (crash reports) and the log file
    }
}
