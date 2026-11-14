// dnd -- drag and drop: widgets as drag sources and drop targets, the hover of a drag over a
// window, drops of other programs onto widgets, dragging out of the app
//
// Sources (the app's own drags):
//   drag_source(id, ev) -> DS_*      after the widget's behave (ev = its EV_*): DS_START when it
//                                    starts being dragged (EV_DRAG; hand the content to drag_set
//                                    in the same pass, else nothing is dragged), DS_ACTIVE while
//                                    it is dragged, DS_END once when the drag ended
//   drag_set(data)                   what is dragged (DragData: text, files, URLs, HTML, a PNG,
//                                    the app's own kind DT_USER0 << k with bytes, a label shown
//                                    next to the pointer, the effects the source allows)
//   drag_out(data) -> started        (left button held) hand data to the platform at once
//   drag_effect() -> DE_*            how the app's last drag ended (DE_COPY / DE_MOVE /
//                                    DE_LINK; 0 = cancelled or refused); UIC_DRAG_END says it too
//   dragging() -> bool               one of the app's drags runs
// Targets:
//   drop_target(id, types) -> DS_*   the widget at last_rect takes drags of these DT_* kinds:
//                                    DS_HOVER while one is over it (the style highlights it),
//                                    DS_DROP in the pass of the drop (drop_data(), drop_effect():
//                                    one of the target's effects that the source allows; none in
//                                    common: refused, no DS_DROP)
//   drop_effect_set(effects)         the DE_* effects the target of the last drop_target call
//                                    takes in this pass (default DE_ALL): the platform answers a
//                                    drag over it with one of them (the system's pointer shows
//                                    copy, move or link), 0 refuses the drag there (no drop, no
//                                    highlight); a drag of the app picks from them too
//   drop_accept(types)               what the windows take outside every drop target (default
//                                    DT_FILES); such drops come as UIC_DROP (drop.rs, as before)
//   drag_over() -> (types, x, y)     the drag over the live window: its kinds and position (DIP);
//                                    (0, 0, 0) when none; drag_over_px() the position in px of
//                                    the window, exact (as drop_pos_px, drop.rs)
//   drag_allowed() -> DE_*           the effects its source allows (DE_ALL where the platform does
//                                    not say); after a drop its source's, from E_DROP to the end
//                                    of its passes and while drop_defer waits; 0 when none
//   drop_defer() -> bool             (in the pass of a drop: DS_DROP or UIC_DROP) the drop's answer
//                                    to its source waits for drop_finish (at most DROP_DEFER_MS);
//                                    false: the platform answered at the drop already
//   drop_finish(effect) -> bool      the answer: the DE_* effect it was taken with, 0 = not taken
//
// An internal drag stays in the app while the pointer stays in its window: the drop target under
// the pointer is highlighted, a label follows the pointer, Esc cancels, a release over a matching
// target drops there (else nothing happens).  The target is the innermost drop target at the
// point (the smallest of the topmost layer); targets under a popup or a modal layer do not count.
// When the pointer leaves the window with content other programs know (DT_STD), the platform
// takes over (plat::drag_start: DoDragDrop, Xdnd, wl_data_device.start_drag); dropped back onto
// the app the drop carries the app's own content again (its kind and bytes).  Headless runs keep
// what would have left the app for the scripts (`dragout expect`, `dragout end`).
// Other programs' drags: E_DRAG (hover: the targets highlight), E_DROP (onto the drop target at
// the point, else UIC_DROP when drop_accept takes it).  The platform asks the published targets
// synchronously (uiw_drag_pw, win.rs) to answer the source with an effect: the effects the target
// under the point took in the last pass (drop_effect_set), DE_ALL for drop_accept's window --
// IDropTarget::DragOver's pdwEffect, XdndStatus' action, wl_data_offer.set_actions, macOS'
// NSDragOperation (plat::drag_query).  The answer follows the app's pass: an effect set for the
// drag's point in one pass answers the moves until the next pass.  The effects the source allows
// come with E_DRAG (forgotten when it leaves or the drag ends) and with the drop (plat::
// drop_allowed; without them those of the hover before it, else every effect): the drop's effect
// is one the target takes that the source allows -- the platform's, else the first of copy,
// move, link -- and a target that takes none of them refuses the drop as the platform refuses
// such a drag over it.
// The answer of a drop to its source (the effect, or that it was not taken) goes after the
// passes of E_DROP: the effect it was delivered with -- unless the app called drop_defer in them
// and answers later with drop_finish (a web page decides asynchronously whether it took a drop).
// Where the platform allows a later answer (plat::drop_hold: X11, Wayland, Windows with a source
// in async mode) it then reaches the source; elsewhere drop_defer says false and the source got
// the effect at the drop.  A deferred drop without drop_finish is answered after DROP_DEFER_MS as
// it was delivered (a UIWARN); so is one a newer drop overtakes.  Headless runs keep the answer
// for the scripts (`drop expect`).
// Positions stay logical in mirrored (RTL) windows: E_DRAG / E_DROP points are mirrored on the
// way in like every pointer event; the label next to the pointer is drawn through the mirror.

use alloc::string::String;

use super::consts::*;
use super::keys::*;
use super::{Sp, Ui};
use plat::{DE_ALL, DE_COPY, DE_MOVE, DT_FILES, DT_STD, DragData, Event};

const UIT_DROP: u32 = 0x7FFE_0060; // timer: a deferred drop's time is up

const PH_NONE: u8 = 0;
const PH_START: u8 = 1; // drag_source said DS_START, drag_set not called yet
const PH_IN: u8 = 2; // inside the source window
const PH_OUT: u8 = 3; // the platform carries it (or, headless, it left the window)
const LABEL_MAX: usize = 48; // characters of the label next to the pointer

/// a drop target of a pass (clipped, physical px of the window; x1 / y1 exclusive)
#[derive(Clone, Copy, Default)]
pub(crate) struct Zone {
    pub(crate) id: u32,
    pub(crate) types: u32,
    /// the DE_* effects it takes (drop_effect_set; DE_ALL by default, 0 refuses)
    pub(crate) effects: u32,
    pub(crate) layer: u8,
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) x1: i32,
    pub(crate) y1: i32,
}

/// the drop target at (x, y) that takes one of `types`: of the topmost layer that has a hit
/// box or a target there (never below `modal`), the smallest one
pub(crate) fn zone_find(zones: &[Zone], hits: &[super::ctx::Hit], modal: u32, x: i32, y: i32, types: u32) -> Option<Zone> {
    if types == 0 || (x == -1 && y == -1) {
        return None;
    }
    let inside = |x0: i32, y0: i32, x1: i32, y1: i32| x >= x0 && x < x1 && y >= y0 && y < y1;
    // a popup over the target hides it
    let top = hits.iter().filter(|h| inside(h.x0, h.y0, h.x1, h.y1)).map(|h| h.layer as u32).max().unwrap_or(0).max(modal);
    let area = |z: &Zone| (z.x1 - z.x0) as i64 * (z.y1 - z.y0) as i64;
    let mut best: Option<Zone> = None;
    for z in zones.iter().filter(|z| (z.layer as u32) >= top && z.types & types != 0 && inside(z.x0, z.y0, z.x1, z.y1)) {
        let better = match &best {
            None => true,
            Some(b) => z.layer > b.layer || (z.layer == b.layer && area(z) <= area(b)),
        };
        if better {
            best = Some(*z);
        }
    }
    best
}

/// what the platform's hook answers at (x, y) logical px of a window (its targets, hit boxes and
/// modal layer of the last pass) for a drag offering `offered` (DT_STD bits) -> (accepted DT_*,
/// DE_* effects): the drop target there with its effects, else `accept` with every effect.  `own`:
/// the kinds of the app's own drag while the platform carries it
#[allow(clippy::too_many_arguments)]
pub(crate) fn drag_answer(zones: &[Zone], hits: &[super::ctx::Hit], modal: u32, accept: u32, own: u32, x: i32, y: i32, offered: u32) -> (u32, u32) {
    let offered = offered & DT_STD;
    let all = offered | own; // the app's own drag: its own kinds too
    let (t, fx) = match zone_find(zones, hits, modal, x, y, all) {
        Some(z) => (z.types & all, z.effects & DE_ALL),
        None => (accept & all, DE_ALL),
    };
    // the platform knows the standard kinds: a target of the app's own kind takes them all
    let t = if t != 0 && t & offered == 0 { offered } else { t & offered };
    if t == 0 || fx == 0 { (0, 0) } else { (t, fx) }
}

/// a drop whose answer to its source has not gone yet
#[derive(Clone, Copy)]
struct Wait {
    /// plat::drop_waiting's serial (headless: one of our own)
    serial: u32,
    /// the effect it was delivered with (0: refused): the answer without drop_finish
    fallback: u32,
    /// the effects its source allows (files never moved)
    allowed: u32,
    /// drop_defer was called: when (ms)
    deferred: Option<u32>,
}

/// the drag and drop state shared by the windows
pub(crate) struct S {
    phase: u8,
    src: u32,
    src_win: u32,
    /// what the app drags
    data: DragData,
    /// DS_END for `end_src` in the next pass
    end_src: u32,
    end_win: u32,
    end_live: bool,
    last_effect: u32,
    /// another program's drag over window ext_win (logical px)
    ext: bool,
    ext_win: u32,
    ext_x: i32,
    ext_y: i32,
    ext_types: u32,
    /// the DE_* effects its source allows (E_DRAG; 0: not said); forgotten when it leaves, is
    /// dropped or the app's own drag ends
    ext_allowed: u32,
    /// the effects the source of the last drop allows (0: nobody said) and that its passes run
    /// (E_DROP .. dnd_drop_settled): drag_allowed then, and while its deferred answer waits
    drop_allowed: u32,
    drop_now: bool,
    /// the zone of the last drop_target call (window, index in its pass's list, id), for
    /// drop_effect_set
    last_zone: Option<(u32, usize, u32)>,
    /// the drop target under the drag (this pass) and its highlight (window, layer, rect),
    /// drawn at the end of the window's pass over the target's content
    over: u32,
    over_win: u32,
    hl: Option<(u32, u8, [i32; 4])>,
    /// DS_DROP for drop_id of drop_win in the next pass
    drop_id: u32,
    drop_win: u32,
    drop_live: bool,
    pub(crate) drop_effect: u32,
    /// the content of the last drop
    pub(crate) dropped: DragData,
    /// what a window takes outside the drop targets
    pub(crate) accept: u32,
    /// headless: what a drag out handed over (scripts)
    pub(crate) out: Option<DragData>,
    /// the last drop's answer to its source while it has not gone; what it was once it went
    /// (None: none yet) -- `drop expect`
    wait: Option<Wait>,
    pub(crate) answered: Option<u32>,
    /// headless: the next drop is answered at its drop, as on a platform without a later answer
    /// (script `drop ... sync`); the serials of the headless drops
    pub(crate) sync_next: bool,
    serial: u32,
}

impl Default for S {
    fn default() -> S {
        S {
            phase: PH_NONE,
            src: 0,
            src_win: 0,
            data: DragData::default(),
            end_src: 0,
            end_win: 0,
            end_live: false,
            last_effect: 0,
            ext: false,
            ext_win: 0,
            ext_x: 0,
            ext_y: 0,
            ext_types: 0,
            ext_allowed: 0,
            drop_allowed: 0,
            drop_now: false,
            last_zone: None,
            over: 0,
            over_win: 0,
            hl: None,
            drop_id: 0,
            drop_win: 0,
            drop_live: false,
            drop_effect: 0,
            dropped: DragData::default(),
            accept: DT_FILES,
            out: None,
            wait: None,
            answered: None,
            sync_next: false,
            serial: 0,
        }
    }
}

// the label next to the pointer: the app's, else from the content
fn label_of(d: &DragData) -> String {
    let cut = |s: &str| -> String {
        let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let mut out: String = line.chars().take(LABEL_MAX).collect();
        if line.chars().count() > LABEL_MAX {
            out.push('…');
        }
        out
    };
    if !d.label.is_empty() {
        return cut(&d.label);
    }
    if let Some(t) = &d.text {
        return cut(t);
    }
    if let Some(f) = d.files.first() {
        let name = f.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(f);
        let mut s = cut(name);
        if d.files.len() > 1 {
            s.push_str(&alloc::format!(" +{}", d.files.len() - 1));
        }
        return s;
    }
    if let Some(u) = d.urls.first() {
        return cut(u);
    }
    String::new()
}

impl Ui {
    // ------------------------------------------------------------------ sources

    /// drag_source(id, ev) -> DS_START / DS_ACTIVE / DS_END / 0 for the widget `id` whose
    /// behave returned ev: DS_START when a drag of it begins (call drag_set in this pass),
    /// DS_ACTIVE while its content is dragged, DS_END once afterwards (drag_effect())
    pub fn drag_source(&mut self, id: u32, ev: u32) -> u32 {
        let cur = self.uiw_cur;
        let d = &self.dnd;
        if id == 0 {
            return 0;
        }
        if d.end_live && d.end_src == id && d.end_win == cur {
            return DS_END;
        }
        if d.phase != PH_NONE && d.src == id && d.src_win == cur {
            return if d.phase == PH_START { DS_START } else { DS_ACTIVE };
        }
        if d.phase == PH_NONE && ev & EV_DRAG != 0 && self.wp.ctx.active == id && self.wp.ctx.mbtn & (1 << BTN_LEFT) != 0 {
            let d = &mut self.dnd;
            d.phase = PH_START;
            d.src = id;
            d.src_win = cur;
            d.data = DragData::default();
            self.wp.ctx.active = 0; // the drag owns the pointer: the release is no click
            return DS_START;
        }
        0
    }

    /// drag_set(data): the content of the drag that starts (after DS_START, same pass)
    pub fn drag_set(&mut self, data: DragData) {
        let d = &mut self.dnd;
        if d.phase == PH_START && d.src_win == self.uiw_cur && !data.is_empty() {
            d.data = data;
            d.phase = PH_IN;
            self.repass = true; // the targets see it in this frame
        }
    }

    /// drag_out(data) -> started: (the left button held) the platform drags data out of the
    /// app right away (DoDragDrop, Xdnd, wl_data_device.start_drag); UIC_DRAG_END at the end
    pub fn drag_out(&mut self, data: DragData) -> bool {
        if self.dnd.phase != PH_NONE || data.types() & DT_STD == 0 {
            return false;
        }
        self.dnd.src = 0;
        self.dnd.src_win = self.uiw_cur;
        self.dnd.data = data;
        self.dnd_go_out()
    }

    /// the effect of the app's last drag: DE_COPY / DE_MOVE / DE_LINK, 0 = cancelled / refused
    pub fn drag_effect(&self) -> u32 {
        self.dnd.last_effect
    }

    /// one of the app's drags runs (inside the app or carried by the platform)
    pub fn dragging(&self) -> bool {
        self.dnd.phase >= PH_IN
    }

    // the platform takes the drag over (headless: kept for the scripts) -> it did
    fn dnd_go_out(&mut self) -> bool {
        let data = self.dnd.data.clone();
        // (hot reload: the replay keeps the drag inside; recording, it ends the replays)
        let ok = if self.headless || self.hr_outside("a drag out of the app") {
            self.dnd.out = Some(data);
            true
        } else {
            plat::drag_start(data)
        };
        if ok {
            self.dnd.phase = PH_OUT;
            self.dnd_publish();
        }
        ok
    }

    // the app's drag is over: DS_END in the next pass, UIC_DRAG_END
    fn dnd_end(&mut self, effect: u32) {
        let d = &mut self.dnd;
        d.end_src = d.src;
        d.end_win = d.src_win;
        d.end_live = d.src != 0;
        d.last_effect = effect;
        d.phase = PH_NONE;
        d.src = 0;
        d.data = DragData::default();
        if d.over_win == d.src_win {
            d.over = 0;
        }
        self.dnd_publish();
        self.cmd_run(UIC_DRAG_END, effect);
        self.repass = true;
    }

    // ------------------------------------------------------------------ targets

    /// drop_target(id, types) -> DS_HOVER / DS_DROP / 0: the widget at last_rect takes drags
    /// of these DT_* kinds (DT_USER0 << k: the app's own).  DS_HOVER while such a drag is over
    /// it (the style draws the highlight), DS_DROP in the pass of the drop (drop_data(),
    /// drop_effect(): one of the target's effects that the source allows; none in common: the
    /// drop is refused, no DS_DROP)
    pub fn drop_target(&mut self, id: u32, types: u32) -> u32 {
        if id == 0 || types == 0 {
            return 0;
        }
        let c = &mut self.wp.ctx;
        if c.disabled_depth != 0 {
            return 0;
        }
        let r = c.last_rect;
        let k = c.uiclip;
        let cur = c.hit_cur;
        let mut z = Zone {
            id,
            types,
            effects: DE_ALL,
            layer: c.cmd_layer_ui as u8,
            x0: r[0].max(k[0]),
            y0: r[1].max(k[1]),
            x1: r[0].wrapping_add(r[2]).min(k[2]),
            y1: r[1].wrapping_add(r[3]).min(k[3]),
        };
        let full = c.zones[cur].len() >= super::ctx::HITS_MAX;
        let win = self.uiw_cur;
        if z.x1 > z.x0 && z.y1 > z.y0 && !full {
            self.dnd.last_zone = Some((win, c.zones[cur].len(), id));
            c.zones[cur].push(z);
        } else {
            self.dnd.last_zone = None;
            z.x1 = z.x0; // clipped away (or no room): no highlight
            if full {
                self.limit_hit(super::ctx::LIM_ZONES);
            }
        }
        let d = &self.dnd;
        if d.drop_live && d.drop_win == win && d.drop_id == id {
            return DS_DROP;
        }
        if d.over == id && d.over_win == win && self.dnd_types_in(win) & types != 0 {
            self.dnd.hl = Some((win, z.layer, [z.x0, z.y0, z.x1 - z.x0, z.y1 - z.y0]));
            return DS_HOVER;
        }
        0
    }

    /// drop_effect_set(effects): the DE_* effects (DE_COPY | DE_MOVE | DE_LINK) the drop target
    /// of the last drop_target call takes in this pass; DE_ALL until it is called.  The platform
    /// answers a drag of another program over the target with one of them (the modifier keys
    /// choose, else copy, move, link in this order; files are never moved): the system's pointer
    /// shows that effect and the drop carries it (drop_effect()).  0 refuses the drag there: no
    /// drop, no highlight, the pointer shows "no" -- a page that takes drops only in some places;
    /// so is a drag that allows none of them (a link target, a source that only copies or moves).
    /// A drag of the app inside its window picks from them the same way (a plain drag moves when
    /// both allow it).  Call it after drop_target in every pass the effect should hold, typically
    /// while drop_target says DS_HOVER, for the point of drag_over_px: the platform's answers
    /// follow the passes, one pass late while the drag moves
    pub fn drop_effect_set(&mut self, effects: u32) {
        let (cur, win) = (self.wp.ctx.hit_cur, self.uiw_cur);
        let Some((w, i, id)) = self.dnd.last_zone else { return };
        if w != win {
            return;
        }
        let Some(z) = self.wp.ctx.zones[cur].get_mut(i).filter(|z| z.id == id) else { return };
        z.effects = effects & DE_ALL;
        let r = [z.x0, z.y0, z.x1 - z.x0, z.y1 - z.y0];
        if effects & self.dnd_allowed_in(win) == 0 && self.dnd.over == id && self.dnd.hl.is_some_and(|h| h.0 == win && h.2 == r) {
            self.dnd.hl = None; // a target that refuses the drag over it is not highlighted
        }
    }

    // the DE_* effects the drag over window win allows (0 = none): the app's own its source's
    // (DragData::allowed), another program's what it offers (0 there: copy | move, as
    // plat::drag_decide), never a move of another program's files
    fn dnd_allowed_in(&self, win: u32) -> u32 {
        let d = &self.dnd;
        if d.phase == PH_IN && d.src_win == win {
            d.data.allowed()
        } else if d.ext && d.ext_win == win {
            let a = if d.ext_allowed == 0 { DE_COPY | DE_MOVE } else { d.ext_allowed };
            if d.ext_types & DT_FILES != 0 { a & !DE_MOVE } else { a }
        } else {
            0
        }
    }

    /// drop_accept(types): the DT_* kinds a window takes outside every drop target (default
    /// DT_FILES); those drops arrive as UIC_DROP
    pub fn drop_accept(&mut self, types: u32) {
        if self.dnd.accept != types {
            self.dnd.accept = types;
            self.dnd_publish();
        }
    }

    /// drag_over() -> (DT_* kinds, x, y DIP): the drag over the live window, (0, 0, 0) = none
    pub fn drag_over(&self) -> (u32, i32, i32) {
        self.dnd_over().map_or((0, 0, 0), |(types, x, y)| (types, self.px2dip(x), self.px2dip(y)))
    }

    /// drag_over_px() -> (DT_* kinds, x, y px): drag_over with the position in px of the live
    /// window, exact (logical like `last_rect`; minus a canvas's `UiInput::rect`: the point in
    /// the canvas's own pixels); (0, 0, 0) = none
    pub fn drag_over_px(&self) -> (u32, i32, i32) {
        self.dnd_over().unwrap_or((0, 0, 0))
    }

    /// drag_allowed() -> the DE_* effects (DE_COPY | DE_MOVE | DE_LINK) the source of the drag over
    /// the live window allows: another program's as its platform says -- Windows the effects of its
    /// DoDragDrop, X11 the source's XdndActionList with the action it asks for (DE_ALL without a
    /// list), Wayland its source_actions (copy and move: Wayland has no link), macOS the drag's
    /// operation mask (narrowed by the modifier keys, as AppKit hands it on) -- the app's own drag
    /// its DragData::allowed(); DE_ALL where the platform does not say.  After a drop, the effects
    /// its source allows -- as the platform gives them with the drop (Windows Drop's pdwEffect, X11
    /// the list with the last XdndPosition's action, Wayland source_actions, macOS the operation
    /// mask), else as the hover before it said, else DE_ALL (the browser, Android) -- from its
    /// E_DROP to the end of the passes it starts (DS_DROP, App::command's UIC_DROP) and, when the
    /// app called drop_defer, until its answer went (drop_finish, DROP_DEFER_MS, a newer drop): in
    /// every window over which no drag is, and drop_finish answers within them.  0 otherwise: no
    /// drag over the window, no drop in its passes or waiting.  A page that decides an effect (a
    /// web page's effectAllowed and dropEffect) picks from these, then drop_effect_set; the
    /// platform never moves another program's files whatever this says
    pub fn drag_allowed(&self) -> u32 {
        let win = self.uiw_cur;
        let d = &self.dnd;
        let said = |a: u32| if a & DE_ALL != 0 { a & DE_ALL } else { DE_ALL };
        if d.phase == PH_IN && d.src_win == win {
            d.data.allowed()
        } else if d.ext && d.ext_win == win {
            said(d.ext_allowed)
        } else if d.drop_now || d.wait.is_some_and(|w| w.deferred.is_some()) {
            said(d.drop_allowed)
        } else {
            0
        }
    }

    /// drop_defer() -> waits: (in the pass of a drop: while drop_target says DS_DROP, or in
    /// App::command's UIC_DROP) the drop's answer to its source -- the effect it was taken with,
    /// or that it was not taken -- waits for drop_finish, at most DROP_DEFER_MS; then it goes as
    /// the drop was delivered (drop_effect()) with a UIWARN.  For a target that learns later
    /// whether it took the drop (a web page's drop handler in another process).  false: the answer
    /// went already (the platform answers at the drop: macOS, the browser, Android, Windows with a
    /// source whose data object has no async mode; an internal drag of the app; outside the pass
    /// of a drop): drop_finish has nothing to do then.  What the source sees, per platform: docs/
    /// GUIDE.md section 7, "Drag and drop"
    pub fn drop_defer(&mut self) -> bool {
        let now = self.now_ms;
        let Some(w) = self.dnd.wait.as_mut() else { return false };
        if w.deferred.is_none() {
            w.deferred = Some(now);
            self.timer(UIT_DROP, DROP_DEFER_MS);
        }
        true
    }

    /// drop_finish(effect) -> answered: the answer of the drop that waits (drop_defer; or, in the
    /// pass of a drop, instead of the effect it was delivered with): the DE_* effect it was taken
    /// with, 0 = not taken.  An effect the source does not allow (drag_allowed says them until
    /// then) becomes the first it does (copy, move, link): taken stays taken.  false: no drop
    /// waits (none was deferred, it was answered, or its DROP_DEFER_MS were up)
    pub fn drop_finish(&mut self, effect: u32) -> bool {
        let Some(w) = self.dnd.wait else { return false };
        let e = effect & DE_ALL;
        let e = if e == 0 {
            0
        } else if e & w.allowed != 0 {
            plat::drag_effect_pick(e & w.allowed, 0)
        } else {
            plat::drag_effect_pick(w.allowed, 0)
        };
        self.dnd_answer(e);
        true
    }

    // the drop that waits gets its answer: the platform's source (headless: kept for `drop
    // expect`)
    fn dnd_answer(&mut self, effect: u32) {
        let Some(w) = self.dnd.wait.take() else { return };
        if w.deferred.is_some() {
            self.timers.retain(|t| t.0 != UIT_DROP);
            self.need = true; // drag_allowed says the drop's effects no more
        }
        if !self.headless {
            plat::drop_finish(w.serial, effect);
        }
        self.dnd.answered = Some(effect);
    }

    // a new drop (serial: the platform's while its answer waits, 0: it went at the drop) delivered
    // with `effect`, its source allowing `allowed`: an older one that still waits is answered as
    // it was delivered
    fn dnd_wait_begin(&mut self, serial: u32, effect: u32, allowed: u32) {
        if let Some(w) = self.dnd.wait {
            self.dnd_answer(w.fallback);
        }
        if serial == 0 {
            self.dnd.answered = Some(effect);
            return;
        }
        self.dnd.answered = None;
        self.dnd.wait = Some(Wait { serial, fallback: effect, allowed, deferred: None });
    }

    /// (runtime, after the passes of E_DROP) a drop the app did not defer is answered now; its
    /// passes are over (drag_allowed: the drop's effects only while its deferred answer waits)
    pub(crate) fn dnd_drop_settled(&mut self) {
        if let Some(w) = self.dnd.wait.filter(|w| w.deferred.is_none()) {
            self.dnd_answer(w.fallback);
        }
        if core::mem::take(&mut self.dnd.drop_now) && !self.dnd.wait.is_some_and(|w| w.deferred.is_some()) {
            self.need = true; // a frame without the drop's effects
        }
    }

    /// (runtime, the window loop ends) a drop whose answer still waits: answered as it was
    /// delivered
    pub(crate) fn dnd_quit(&mut self) {
        if let Some(w) = self.dnd.wait {
            self.dnd_answer(w.fallback);
        }
    }

    /// (scripts: `drop expect`) the last drop's answer to its source: None while it waits
    pub(crate) fn dnd_answered(&self) -> Option<Option<u32>> {
        if self.dnd.wait.is_some() { Some(None) } else { self.dnd.answered.map(Some) }
    }

    // the drag over the live window: its kinds and position (px)
    fn dnd_over(&self) -> Option<(u32, i32, i32)> {
        let win = self.uiw_cur;
        let d = &self.dnd;
        if d.phase == PH_IN && d.src_win == win {
            let c = &self.wp.ctx;
            return Some((d.data.types(), c.mx, c.my));
        }
        if d.ext && d.ext_win == win {
            return Some((d.ext_types, d.ext_x, d.ext_y));
        }
        None
    }

    /// the effect of the last drop (DE_*): one its target takes (drop_effect_set; outside the
    /// targets every one) that its source allows (drag_allowed in its passes), files never moved
    /// -- the platform's choice, else the first of copy, move, link.  A drop with none in common
    /// is refused (no DS_DROP, no UIC_DROP; its source is answered "not taken")
    pub fn drop_effect(&self) -> u32 {
        self.dnd.drop_effect
    }

    // the kinds of the drag over window win (0 = none)
    fn dnd_types_in(&self, win: u32) -> u32 {
        let d = &self.dnd;
        if d.phase == PH_IN && d.src_win == win {
            d.data.types()
        } else if d.ext && d.ext_win == win {
            d.ext_types
        } else {
            0
        }
    }

    // ------------------------------------------------------------------ the runtime's calls

    /// uiw_pass_begin: the drop target under the drag (the previous pass's targets)
    pub(crate) fn dnd_pass_begin(&mut self) {
        let win = self.uiw_cur;
        self.dnd.last_zone = None;
        if self.dnd.hl.is_some_and(|h| h.0 == win) {
            self.dnd.hl = None;
        }
        let d = &self.dnd;
        let at = if d.phase == PH_IN && d.src_win == win {
            Some((self.wp.ctx.mx, self.wp.ctx.my, d.data.types()))
        } else if d.ext && d.ext_win == win {
            Some((d.ext_x, d.ext_y, d.ext_types))
        } else {
            None
        };
        if let Some((x, y, types)) = at {
            let c = &self.wp.ctx;
            let z = if self.uiw_inert_now() { None } else { zone_find(&c.zones[c.hit_cur ^ 1], &c.hits[c.hit_cur ^ 1], c.modal_prev, x, y, types) };
            self.dnd.over = z.map_or(0, |z| z.id);
            self.dnd.over_win = win;
        } else if self.dnd.over_win == win {
            self.dnd.over = 0;
        }
    }

    /// uiw_pass_end: the highlight of the drop target under a drag, the label next to the
    /// pointer of the app's drag
    pub(crate) fn dnd_pass_end(&mut self) {
        if let Some((w, layer, r)) = self.dnd.hl {
            if w == self.uiw_cur && r[2] > 0 {
                let prev = self.layer_set(layer as u32);
                let st = self.style;
                st.drop_highlight(self, r);
                self.layer_set(prev);
            }
        }
        let d = &self.dnd;
        if d.phase != PH_IN || d.src_win != self.uiw_cur {
            return;
        }
        let s = label_of(&d.data);
        if s.is_empty() {
            return;
        }
        let (mx, my) = (self.wp.ctx.mx, self.wp.ctx.my);
        if mx == -1 && my == -1 {
            return;
        }
        let lr = self.wp.ctx.last_rect;
        let prev = self.layer_set(L_TIP);
        let st = self.style;
        let mut sp = Sp { label: Some(&s), ..Sp::default() };
        let (w, h) = st.tooltip_size(self, &mut sp);
        // right of and below the pointer, inside the window
        let (fw, fh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let gap = self.dp(12);
        let x = (mx + gap).min(fw - w).max(0);
        let y = (my + gap).min(fh - h).max(0);
        sp.place([x, y, w, h]);
        st.tooltip(self, &sp);
        self.layer_set(prev);
        self.wp.ctx.last_rect = lr;
    }

    /// uiw_clear_edges (after the first pass): DS_DROP / DS_END were seen
    pub(crate) fn dnd_clear_edges(&mut self) {
        self.dnd.drop_live = false;
        self.dnd.end_live = false;
    }

    /// after the passes: a DS_START nobody answered with drag_set is no drag; a drag that
    /// started outside its window (a fast pointer) goes to the platform now
    pub(crate) fn dnd_after_passes(&mut self) {
        // a deferred drop whose time is up: answered as it was delivered
        if let Some(w) = self.dnd.wait {
            if w.deferred.is_some_and(|t| self.now_ms.wrapping_sub(t) >= DROP_DEFER_MS) {
                self.warn("a deferred drop got no drop_finish within DROP_DEFER_MS: answered as it was delivered");
                self.dnd_answer(w.fallback);
            }
        }
        if self.dnd.phase == PH_START {
            self.dnd.phase = PH_NONE;
            self.dnd.src = 0;
        }
        if self.dnd.phase == PH_IN {
            let (cur, w) = (self.uiw_cur, self.dnd.src_win);
            self.uiw_use(w);
            if self.wp.ctx.mbtn & (1 << BTN_LEFT) == 0 {
                self.dnd_end(0); // the button went without a release (focus lost): cancelled
            } else {
                self.dnd_moved();
            }
            self.uiw_use(cur);
        }
    }

    /// needs_move_pass: a drag of the app follows the pointer
    pub(crate) fn dnd_follows_pointer(&self) -> bool {
        self.dnd.phase == PH_IN && self.dnd.src_win == self.uiw_cur
    }

    /// input_event, E_MOVE of the live window: the drag left the window -> the platform
    /// takes over (content other programs know)
    pub(crate) fn dnd_moved(&mut self) {
        if !self.dnd_follows_pointer() || self.dnd.data.types() & DT_STD == 0 {
            return;
        }
        let c = &self.wp.ctx;
        let (x, y) = (c.mx, c.my);
        let fb = &self.wp.rt.fb;
        if x >= 0 && y >= 0 && x < fb.w && y < fb.h {
            return;
        }
        if c.mbtn & (1 << BTN_LEFT) == 0 {
            return;
        }
        if self.dnd_go_out() {
            self.dnd.over = 0;
            self.repass = true;
        }
    }

    /// input_event, E_UP of the left button / Esc: the app's drag inside its window ends
    /// -> true when the event belonged to the drag
    pub(crate) fn dnd_input(&mut self, ev: &Event) -> bool {
        let d = &self.dnd;
        if d.src_win != self.uiw_cur || !(d.phase == PH_START || d.phase == PH_IN) {
            return false;
        }
        match ev.typ {
            plat::E_KEY_DOWN if ev.data & 0xFFFF == VK_ESCAPE => {
                self.dnd_end(0); // Esc cancels
                true
            }
            plat::E_UP if ev.btn == BTN_LEFT as u8 => {
                if self.dnd.phase == PH_START {
                    self.dnd_after_passes();
                    return false;
                }
                // the drop target under the pointer (previous pass)
                let (x, y) = if ev.x != -1 { (ev.x, ev.y) } else { (self.wp.ctx.mx, self.wp.ctx.my) };
                let c = &self.wp.ctx;
                let types = self.dnd.data.types();
                let z = zone_find(&c.zones[c.hit_cur ^ 1], &c.hits[c.hit_cur ^ 1], c.modal_prev, x, y, types);
                let mut effect = 0;
                if let Some(z) = z {
                    // inside the app a plain drag moves (when the source and the target allow
                    // it); a target that takes none of the source's effects refuses it
                    let a = self.dnd.data.allowed() & z.effects;
                    effect = if self.mods & (MOD_CTRL | MOD_SHIFT) == 0 && a & DE_MOVE != 0 {
                        DE_MOVE
                    } else {
                        plat::drag_effect_pick(a, self.mods as u8)
                    };
                    if effect != 0 {
                        let win = self.uiw_cur;
                        let mut data = self.dnd.data.clone();
                        data.effects = effect;
                        self.dnd_deliver(z.id, win, x, y, data);
                    }
                }
                self.dnd_end(effect);
                false // the release still reaches the widgets
            }
            _ => false,
        }
    }

    // a drop for target id of window win at (x, y) logical px: DS_DROP in the next pass
    fn dnd_deliver(&mut self, id: u32, win: u32, x: i32, y: i32, data: DragData) {
        let files = data.files.clone();
        self.dr.list = files;
        self.dr.win = self.uiw_rec_id(win);
        self.dr.px = (x, y);
        self.dr.x = self.px2dip_win(win, x);
        self.dr.y = self.px2dip_win(win, y);
        let d = &mut self.dnd;
        d.drop_effect = data.effects;
        d.dropped = data;
        d.drop_id = id;
        d.drop_win = win;
        d.drop_live = true;
        self.repass = true;
    }

    // px -> DIP in window win
    fn px2dip_win(&mut self, win: u32, v: i32) -> i32 {
        let cur = self.uiw_cur;
        self.uiw_use(win);
        let r = self.px2dip(v);
        self.uiw_use(cur);
        r
    }

    /// E_DRAG of window win (UI index): another program's drag (or the app's own one coming
    /// back) is over it at (x, y) px; data 0: it left
    pub(crate) fn dnd_hover(&mut self, ev: &Event) {
        let win = ev.win as u32;
        if ev.data == 0 {
            if self.dnd.ext_win == win {
                self.dnd.ext = false;
                self.dnd.ext_allowed = 0; // the next drag says its own
            }
            return;
        }
        let mut types = ev.data & DT_STD;
        if self.dnd.phase == PH_OUT {
            types |= self.dnd.data.types(); // the app's own drag: its own kinds too
        }
        let cur = self.uiw_cur;
        self.uiw_use(win);
        let x = if self.rtl { self.wp.rt.fb.w - 1 - ev.x } else { ev.x }; // logical
        self.uiw_use(cur);
        let d = &mut self.dnd;
        d.ext = true;
        d.ext_win = win;
        d.ext_x = x;
        d.ext_y = ev.y;
        d.ext_types = types;
        d.ext_allowed = (ev.data >> 8) & DE_ALL;
    }

    /// (scripts: `dragover expect`) what the platform would answer the other program's drag over
    /// its window now, with the modifier keys mods (plat::drag_decide over the targets of the
    /// last pass, as uiw_drag_pw does with the published ones) -> (DT_* taken, the DE_* effect,
    /// every effect taken); None: no drag hovers
    pub(crate) fn dnd_answer_now(&mut self, mods: u8) -> Option<(u32, u32, u32)> {
        let d = &self.dnd;
        if !d.ext {
            return None;
        }
        let (win, x, y, offered, allowed) = (d.ext_win, d.ext_x, d.ext_y, d.ext_types & DT_STD, d.ext_allowed);
        let own = if d.phase == PH_OUT { d.data.types() } else { 0 };
        let accept = d.accept;
        let cur = self.uiw_cur;
        self.uiw_use(win);
        let c = &self.wp.ctx;
        let (t, e) = if self.uiw_inert_now() {
            (0, 0)
        } else {
            drag_answer(&c.zones[c.hit_cur ^ 1], &c.hits[c.hit_cur ^ 1], c.modal_prev, accept, own, x, y, offered)
        };
        self.uiw_use(cur);
        Some(plat::drag_decide(t, e, offered, allowed, mods))
    }

    /// E_DROP (drop.rs): the drop of another program (or the app's own drag) at (x, y) logical
    /// px of window win -> true when a drop target took it or it was refused (else UIC_DROP
    /// decides).  serial: the platform's while the drop's answer waits (plat::drop_waiting), 0
    /// when it went at the drop; given: the effects its source allows as the platform gave them
    /// with the drop (plat::drop_allowed; 0: not said)
    pub(crate) fn dnd_dropped(&mut self, win: u32, x: i32, y: i32, mut data: DragData, serial: u32, given: u32) -> bool {
        // what the source allows: the app's own drag its data's; another program's as the drop
        // says, else as the hover over the window before it said (the same drag), else nobody
        // said (every effect)
        let d = &self.dnd;
        let src = if d.phase == PH_OUT {
            d.data.allowed()
        } else if given & DE_ALL != 0 {
            given & DE_ALL
        } else if d.ext && d.ext_win == win {
            d.ext_allowed & DE_ALL
        } else {
            0
        };
        let mut allowed = if src == 0 { DE_ALL } else { src };
        self.dnd.ext = false;
        self.dnd.ext_allowed = 0;
        self.dnd.drop_allowed = src;
        self.dnd.drop_now = true;
        if self.dnd.phase == PH_OUT {
            // the app's own drag came back: its full content
            let effect = data.effects;
            data = self.dnd.data.clone();
            data.effects = effect;
        }
        if data.effects & DE_ALL == 0 || (data.effects == DE_MOVE && !data.files.is_empty()) {
            data.effects = DE_COPY; // files are never moved (plat::drag_query)
        }
        if !data.files.is_empty() {
            allowed &= !DE_MOVE; // files are never moved
        }
        let types = data.types();
        let cur = self.uiw_cur;
        self.uiw_use(win);
        let c = &self.wp.ctx;
        let z = if self.uiw_inert_now() { None } else { zone_find(&c.zones[c.hit_cur ^ 1], &c.hits[c.hit_cur ^ 1], c.modal_prev, x, y, types) };
        self.uiw_use(cur);
        let serial = if self.headless && serial == 0 && !core::mem::take(&mut self.dnd.sync_next) {
            self.dnd.serial = self.dnd.serial.wrapping_add(1).max(1); // headless: a platform that waits
            self.dnd.serial
        } else {
            serial
        };
        // the effects the drop may have there: the target's (drop_effect_set; the window's
        // drop_accept outside them: every one) that the source allows, files never moved.  The
        // platform picked one of them already; a scripted drop (or one that raced the pass) with
        // another effect gets the first of them, and with none in common nothing happens (no
        // DS_DROP, no UIC_DROP; the source is answered "not taken"), as the platform refuses
        // such a drag over the target
        let fx = z.map_or(DE_ALL, |z| z.effects) & allowed;
        if data.effects & fx == 0 {
            data.effects = plat::drag_effect_pick(fx, 0);
        }
        let e = data.effects;
        match z {
            Some(z) => {
                if e != 0 {
                    self.dnd.drop_effect = e;
                    self.dnd_deliver(z.id, win, x, y, data);
                }
                self.dnd_wait_begin(serial, e, allowed);
                true
            }
            None => {
                if e != 0 {
                    self.dnd.drop_effect = e;
                    self.dnd.dropped = data;
                }
                self.dnd_wait_begin(serial, e, allowed);
                e == 0
            }
        }
    }

    /// E_DRAG_END: the platform finished the app's drag (data = the effect)
    pub(crate) fn dnd_out_end(&mut self, effect: u32) {
        if self.dnd.phase == PH_OUT {
            self.dnd.out = None;
            // its hover over a window of the app (it came back) ended with it
            self.dnd.ext = false;
            self.dnd.ext_allowed = 0;
            self.dnd_end(effect & DE_ALL);
        }
    }

    /// (scripts) the content a headless drag out handed over
    pub(crate) fn dnd_out_data(&self) -> Option<&DragData> {
        if self.dnd.phase == PH_OUT { self.dnd.out.as_ref() } else { None }
    }

    // what the platform's synchronous hook needs: the accepted kinds and the app's own drag
    fn dnd_publish(&self) {
        let own = if self.dnd.phase == PH_OUT { self.dnd.data.types() } else { 0 };
        super::win::uiw_publish_dnd(self.dnd.accept, own);
    }
}
