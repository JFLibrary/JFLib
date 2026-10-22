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
//                                    DS_DROP in the pass of the drop (drop_data(), drop_effect())
//   drop_accept(types)               what the windows take outside every drop target (default
//                                    DT_FILES); such drops come as UIC_DROP (drop.rs, as before)
//   drag_over() -> (types, x, y)     the drag over the live window: its kinds and position (DIP);
//                                    (0, 0, 0) when none
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
// synchronously (uiw_drag_pw, win.rs) to answer the source with an effect.
// Positions stay logical in mirrored (RTL) windows: E_DRAG / E_DROP points are mirrored on the
// way in like every pointer event; the label next to the pointer is drawn through the mirror.

use alloc::string::String;

use super::consts::*;
use super::keys::*;
use super::{Sp, Ui};
use plat::{DE_ALL, DE_COPY, DE_MOVE, DT_FILES, DT_STD, DragData, Event};

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
        let ok = if self.headless {
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
    /// it (the style draws the highlight), DS_DROP in the pass of the drop (drop_data())
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
            layer: c.cmd_layer_ui as u8,
            x0: r[0].max(k[0]),
            y0: r[1].max(k[1]),
            x1: r[0].wrapping_add(r[2]).min(k[2]),
            y1: r[1].wrapping_add(r[3]).min(k[3]),
        };
        let full = c.zones[cur].len() >= super::ctx::HITS_MAX;
        if z.x1 > z.x0 && z.y1 > z.y0 && !full {
            c.zones[cur].push(z);
        } else {
            z.x1 = z.x0; // clipped away (or no room): no highlight
            if full {
                self.limit_hit(super::ctx::LIM_ZONES);
            }
        }
        let win = self.uiw_cur;
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
        let win = self.uiw_cur;
        let d = &self.dnd;
        if d.phase == PH_IN && d.src_win == win {
            let c = &self.wp.ctx;
            return (d.data.types(), self.px2dip(c.mx), self.px2dip(c.my));
        }
        if d.ext && d.ext_win == win {
            return (d.ext_types, self.px2dip(d.ext_x), self.px2dip(d.ext_y));
        }
        (0, 0, 0)
    }

    /// the effect of the last drop (DE_*)
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
                    // inside the app a plain drag moves (when the source allows it)
                    let a = self.dnd.data.allowed();
                    effect = if self.mods & (MOD_CTRL | MOD_SHIFT) == 0 && a & DE_MOVE != 0 {
                        DE_MOVE
                    } else {
                        plat::drag_effect_pick(a, self.mods as u8)
                    };
                    let win = self.uiw_cur;
                    let mut data = self.dnd.data.clone();
                    data.effects = effect;
                    self.dnd_deliver(z.id, win, x, y, data);
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
    }

    /// E_DROP (drop.rs): the drop of another program (or the app's own drag) at (x, y) logical
    /// px of window win -> true when a drop target took it (else UIC_DROP decides)
    pub(crate) fn dnd_dropped(&mut self, win: u32, x: i32, y: i32, mut data: DragData) -> bool {
        self.dnd.ext = false;
        if self.dnd.phase == PH_OUT {
            // the app's own drag came back: its full content
            let effect = data.effects;
            data = self.dnd.data.clone();
            data.effects = effect;
        }
        if data.effects & DE_ALL == 0 || (data.effects == DE_MOVE && !data.files.is_empty()) {
            data.effects = DE_COPY; // files are never moved (plat::drag_query)
        }
        let types = data.types();
        let cur = self.uiw_cur;
        self.uiw_use(win);
        let c = &self.wp.ctx;
        let z = if self.uiw_inert_now() { None } else { zone_find(&c.zones[c.hit_cur ^ 1], &c.hits[c.hit_cur ^ 1], c.modal_prev, x, y, types) };
        self.uiw_use(cur);
        self.dnd.drop_effect = data.effects;
        match z {
            Some(z) => {
                self.dnd_deliver(z.id, win, x, y, data);
                true
            }
            None => {
                self.dnd.dropped = data;
                false
            }
        }
    }

    /// E_DRAG_END: the platform finished the app's drag (data = the effect)
    pub(crate) fn dnd_out_end(&mut self, effect: u32) {
        if self.dnd.phase == PH_OUT {
            self.dnd.out = None;
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
