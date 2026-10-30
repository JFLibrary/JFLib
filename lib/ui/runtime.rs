// runtime -- the command line, window or headless run, event loop, passes, rendering
//                                                                     [src/ui/ui_main.s]
//
//   run(desc, app, style) -> exit code     jfgui_main of an app
//   fb() -> &Surface   dpi() -> i32   dp(dip)   px2dip(px)   headless() -> bool
//   invalidate()   quit()   client_size() -> (w, h) DIP   set_title(s)   title() -> &str
// Runtime internals used by the script runner (auto.rs): run_passes, render,
// process_events, resize(w_px, h_px).
//
// Command line (every app):
//   --shot FILE.png     headless: render the first frame, write a PNG, exit
//   --script FILE       headless: run a UI test script (auto.rs)
//   --dump              headless: print the widget tree (JSON lines) after --shot
//   --size WxH          client size in DIP (default: the AppDesc)
//   --scale PERCENT     headless scale (100 = 96 dpi)
//   --theme light|dark  force a theme (default: AppDesc flags, else the system)
//   --accent RRGGBB     accent colour (default: the AppDesc, else the system's)
//   --anim off          no animations (values jump to their targets; default: the system's
//                       "reduce motion")
//   --contrast high|normal   the high-contrast theme on / off (default: the system)
//   --text-scale PERCENT     text sizes and line heights (default: the system's text scale)
//   --rtl / --ltr       mirror the windows / never (default: the UI language)
//   --log-events        print every platform event
//   --auto-close MS     close the window after MS milliseconds (smoke tests)
//   --verify-dirty      after every partial redraw compare with a full redraw
//   --inspect           the live inspector open (inspect.rs; jfgui_INSPECT=1: its hot keys only)
//   (jfgui_HOTRELOAD=DIR: hot reload's session of `build_rs.py watch`, development builds:
//   the input recorded and replayed at the next start, hotreload.rs)
//
// The frame loop: platform events are applied to the input state one "edge" (button,
// key, wheel) at a time; each edge runs App::frame (plus up to three settle passes when
// the layout moved); commands queued by widgets are delivered to App::command in between;
// then the recorded draw commands are rendered and presented.  Async tasks (ui.spawn,
// tasks.rs) run after the commands of a round; their timers bound the wait.  Nothing runs
// while the app is idle.  Several windows (win.rs): one App::frame builds all of them; every
// event carries its window (Event::win).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::consts::*;
use super::{App, AppDesc, Style, Ui, UiInput, WinParts};
use gfx::{Dc, Surface, FMT_XRGB};
use plat::Event;
use text::Text;

const MAX_PASSES: u32 = 4;

/// Launch the same owned app on a desktop event loop, browser worker or Android host.
/// Host-driven applications remain alive after their entrypoint returns.
pub fn launch(desc: &AppDesc, app: impl App + 'static, style: &'static dyn Style) -> i32 {
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    { let mut app = app; run(desc, &mut app, style) }
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    {
        let (mut ui, w, h) = prepare(desc, style);
        plat::win_hit_hook(Some(super::win::uiw_hit_pw));
        plat::drag_hook(Some(super::win::uiw_drag_pw));
        plat::set_min_size(desc.min_w as i32, desc.min_h as i32);
        if !plat::window_create(desc.title, w, h, desc.flags) { return 2; }
        ui.win_hooks_set(true);
        ui.wp.rt.dpi = plat::dpi();
        let (w, h) = plat::window_size();
        ui.wp.rt.fb = Surface::new(w, h, FMT_XRGB);
        ui.set_time(0);
        super::perm::ask_here(true); // permission questions: a dialog in the page (perm.rs)
        let mut app: alloc::boxed::Box<dyn App> = alloc::boxed::Box::new(app);
        if ui.wp.rt.fb.px.is_empty() || !app.init(&mut ui) { return 2; }
        ui.set_agent_enabled(Args(plat::args()).flag("--agent"));
        ui.run_passes(app.as_mut());
        ui.render();
        ui.present();
        let runtime = Runtime { ui, app, finished: false };
        if plat::web::install(alloc::boxed::Box::new(HostRuntime {
            runtime, sessions: BTreeMap::new(), session_clock: 0,
        })) { 0 } else { 2 }
    }
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
struct HostRuntime {
    runtime: Runtime,
    sessions: BTreeMap<String, (u64, agent_core::RpcSession)>,
    session_clock: u64,
}
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
impl plat::web::WebApplication for HostRuntime {
    fn frame_delay(&self) -> u32 {
        let ui = &self.runtime.ui;
        if ui.need || ui.need_render || ui.animating || ui.tasks_ready() { return 0; }
        let task = ui.tasks_deadline().map(|t| (t.saturating_sub(plat::task::now_us()) / 1000).min(250) as i32);
        ui.timers.iter()
            .map(|(_, deadline)| deadline.wrapping_sub(ui.now_ms) as i32)
            .chain(task)
            .min().unwrap_or(250).clamp(0, 250) as u32
    }
    fn input(&mut self, time_ms: u32) {
        if self.runtime.finished { return; }
        self.runtime.ui.set_time(time_ms);
        self.runtime.ui.process_events_deferred(self.runtime.app.as_mut(), false);
        // Resize/drop/wake queue commands rather than an ordinary input edge. Settle
        // their layout before routing the following pointer/key event in this batch.
        if self.runtime.ui.need {
            self.runtime.ui.need = false;
            self.runtime.ui.run_passes(self.runtime.app.as_mut());
        }
    }
    fn set_control_enabled(&mut self, enabled: bool) {
        self.runtime.set_agent_enabled(enabled);
        if !enabled { self.sessions.clear(); }
    }
    fn tick(&mut self, time_ms: u32) {
        self.runtime.step(time_ms, &[]);
        self.runtime.ui.present();
    }
    fn control(&mut self, line: &[u8]) -> Option<String> {
        let envelope = agent_core::json::parse(line).ok()?;
        let session = envelope.get("session").and_then(agent_core::Value::as_str).unwrap_or("page");
        if session.is_empty() || session.len() > 128 { return None; }
        if envelope.get("close").and_then(agent_core::Value::as_bool) == Some(true) {
            self.sessions.remove(session);
            return None;
        }
        let request = envelope.get("request")?;
        let line = agent_core::json::stringify(request).ok()?;
        if !self.sessions.contains_key(session) {
            let Some(protocol) = agent_core::rpc::requested_protocol(line.as_bytes()) else {
                // Report an invalid envelope without binding an otherwise unused session ID.
                let mut transient = agent_core::RpcSession::new(agent_core::Protocol::Json, self.runtime.ui.desc.title, "1");
                return self.runtime.control(&mut transient, line.as_bytes());
            };
            if self.sessions.len() >= 16 {
                let oldest = self.sessions.iter().filter(|(_, (_, rpc))| !rpc.requires_persistent_session())
                    .min_by_key(|(_, (used, _))| *used).map(|(name, _)| name.clone());
                if let Some(oldest) = oldest {
                    self.sessions.remove(&oldest);
                } else {
                    // Never evict a negotiated legacy connection. Notifications have no reply;
                    // requests retain their ID so the caller receives an immediate error.
                    let id = request.get("id")?.clone();
                    return agent_core::json::stringify(&agent_core::Value::object([
                        ("jsonrpc", agent_core::Value::string("2.0")), ("id", id),
                        ("error", agent_core::Value::object([
                            ("code", agent_core::Value::i64(-32000)),
                            ("message", agent_core::Value::string("Browser session limit reached; close an initialized MCP session")),
                        ])),
                    ])).ok();
                }
            }
            self.sessions.insert(String::from(session), (self.session_clock, agent_core::RpcSession::new(protocol, self.runtime.ui.desc.title, "1")));
        }
        self.session_clock = self.session_clock.wrapping_add(1);
        let (used, rpc) = self.sessions.get_mut(session).unwrap();
        *used = self.session_clock;
        let response = self.runtime.control(rpc, line.as_bytes());
        self.runtime.ui.present();
        response
    }
}

/// An owned application driven one event/tick at a time. Browser hosts and automation
/// use the same passes, input handling and software renderer as native windows.
pub struct Runtime {
    pub(crate) ui: Ui,
    pub(crate) app: alloc::boxed::Box<dyn App>,
    finished: bool,
}
impl Runtime {
    pub(crate) fn is_finished(&self) -> bool { self.finished }
    /// Initialize a headless surface with the normal command-line/theme configuration.
    /// `Err(2)` means application initialization or framebuffer allocation failed.
    pub fn new(desc: &AppDesc, app: impl App + 'static, style: &'static dyn Style) -> Result<Self, i32> {
        let (mut ui, w, h) = prepare(desc, style);
        ui.headless = true;
        ui.wp.rt.fb = Surface::new(ui.dp(w), ui.dp(h), FMT_XRGB);
        ui.set_time(0);
        let mut app: alloc::boxed::Box<dyn App> = alloc::boxed::Box::new(app);
        if ui.wp.rt.fb.px.is_empty() || !app.init(&mut ui) { return Err(2); }
        ui.run_passes(app.as_mut());
        ui.render();
        Ok(Self { ui, app, finished: false })
    }
    /// Apply input edges and due timers, then render if required. False means closed.
    /// Time is milliseconds since initialization; it must advance monotonically.
    pub fn step(&mut self, now_ms: u32, events: &[Event]) -> bool {
        if self.finished { return false; }
        self.ui.set_time(now_ms);
        for event in events {
            if !plat::evq_push(*event) {
                self.ui.process_events(self.app.as_mut());
                plat::evq_push(*event);
            }
        }
        self.ui.process_events(self.app.as_mut());
        self.ui.tick_timers();
        if self.ui.need || self.ui.animating || self.ui.tasks_ready() {
            self.ui.need = false;
            self.ui.run_passes(self.app.as_mut());
        }
        self.ui.render();
        if self.ui.quit { self.finish(); }
        !self.finished
    }
    /// Current framebuffer for presentation or screenshot encoding.
    pub fn frame(&self) -> &Surface { self.ui.fb() }
    /// Read the runtime context, including dimensions and semantic widgets.
    pub fn ui(&self) -> &Ui { &self.ui }
    /// Deliver a registered numeric command through the normal application callback.
    pub fn command(&mut self, id: u32, arg: u32) -> bool {
        if self.finished { return false; }
        let handled = self.app.command(&mut self.ui, id, arg);
        self.ui.invalidate();
        handled
    }
    /// Close exactly once and deliver the existing application exit callback.
    pub fn finish(&mut self) {
        if !self.finished {
            self.finished = true;
            self.ui.set_agent_enabled(false);
            self.app.command(&mut self.ui, UIC_EXIT, 0);
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) { self.finish(); }
}

/// the per-window part of the runtime (ui_main_wb0 .. ui_main_wb1): framebuffer, shadow
/// surface of --verify-dirty, dpi, the pending present
pub(crate) struct W {
    pub(crate) fb: Surface,
    pub(crate) shadow: Surface,
    pub(crate) dpi: i32,
    /// rectangles of the last render to present (-1 = everything, 0 = nothing)
    pub(crate) present_n: i32,
}

impl W {
    pub(crate) fn new() -> W {
        W { fb: Surface::new(0, 0, FMT_XRGB), shadow: Surface::new(0, 0, FMT_XRGB), dpi: 96, present_n: 0 }
    }
}

// ------------------------------------------------------------------ command line helpers

/// parse_int (str.s): leading spaces, optional sign, decimal digits -> (value, rest)
pub(crate) fn parse_int(s: &[u8]) -> (i32, &[u8]) {
    let mut i = 0;
    while i < s.len() && s[i] == b' ' {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && s[i] == b'-' {
        neg = true;
        i += 1;
    } else if i < s.len() && s[i] == b'+' {
        i += 1;
    }
    let mut v: i32 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add((s[i] - b'0') as i32);
        i += 1;
    }
    (if neg { v.wrapping_neg() } else { v }, &s[i..])
}

/// parse_hex (str.s): optional "#" or "0x", hex digits -> (value, rest)
pub(crate) fn parse_hex(s: &[u8]) -> (u32, &[u8]) {
    let mut i = 0;
    if !s.is_empty() && s[0] == b'#' {
        i = 1;
    } else if s.len() >= 2 && s[0] == b'0' && s[1] | 32 == b'x' {
        i = 2;
    }
    let mut v: u32 = 0;
    while i < s.len() {
        let d = match s[i] {
            c @ b'0'..=b'9' => c - b'0',
            c => {
                let l = c | 32;
                if (b'a'..=b'f').contains(&l) { l - b'a' + 10 } else { break }
            }
        };
        v = v << 4 | d as u32;
        i += 1;
    }
    (v, &s[i..])
}

// the command line (argument 0 is the program)
struct Args(Vec<String>);

impl Args {
    fn value(&self, name: &str) -> Option<&str> {
        let i = self.0.iter().skip(1).position(|a| a == name)? + 1;
        self.0.get(i + 1).map(|s| s.as_str())
    }
    fn flag(&self, name: &str) -> bool {
        self.0.iter().skip(1).any(|a| a == name)
    }
    // arg_int: parse_int of the value, the default when the option is missing
    fn int(&self, name: &str, def: i32) -> i32 {
        self.value(name).map_or(def, |v| parse_int(v.as_bytes()).0)
    }
}

/// scale_to_dpi(percent): percent clamped to 50..400 -> (p * 96 + 50) / 100
pub(crate) fn scale_to_dpi(p: i32) -> i32 {
    (p.clamp(50, 400) * 96 + 50) / 100
}

impl Ui {
    pub(crate) fn tick_timers(&mut self) {
        loop {
            let (id, _) = self.timer_due();
            if id == u32::MAX { break; }
            self.cmd_run(UIC_TIMER, id);
            self.need = true;
        }
    }
    /// ui_init (+ gfx_init, font_init): the context of a fresh process
    pub(crate) fn new(desc: &AppDesc, style: &'static dyn Style) -> Ui {
        let mut ui = Ui {
            agent: Default::default(),
            desc: *desc,
            style,
            text: Text::new(style.fonts(), style.font_names()),
            dc: Dc::new(0, 0),
            wp: WinParts::new(),
            m: Default::default(),
            sems: Vec::new(),
            semtext_len: 0,
            sem_dummy: Default::default(),
            slots: BTreeMap::new(),
            keydown: [0; 256],
            mpts: Vec::new(),
            mprs: Vec::new(),
            mtilt: [0; 2],
            mpress: 0,
            mptype: 0,
            mpts_win: 0,
            cmdq: alloc::collections::VecDeque::new(),
            text_more: Vec::new(),
            timers: Vec::new(),
            limits: [0; super::ctx::LIM_N],
            mods: 0,
            frame: 0,
            now_ms: 0,
            repass: false,
            animating: false,
            anim_off: false,
            rtl: false,
            rtl_mode: 0,
            app_focus: 0,
            app_active: false,
            theme_sys: false,
            theme_base_dark: false,
            contrast_mode: 0,
            high_contrast: false,
            hc_sys: None,
            accent_sys: false,
            anim_forced: false,
            anim_sys: false,
            text_scale_forced: false,
            text_scale: 100,
            dbl_ms: 500,
            font_prefs: (None, None, None),
            blink_ms: 530,
            warns: 0,
            ui_debug: plat::getenv("jfgui_DEBUG").is_some(),
            dup_check: plat::getenv("jfgui_DEBUG").is_some(),
            dup_warned: alloc::collections::BTreeSet::new(),
            ids: Default::default(),
            focus_req: 0,
            focus_req_win: 0,
            focus_req_old: 0,
            focus_req_frame: 0,
            uiw_cur: 0,
            theme: [0; super::theme::THEME_N],
            theme_dark: false,
            accent: 0,
            look: super::theme::Look::new(style),
            title: String::new(),
            t0_us: 0,
            headless: false,
            need: false,
            need_render: false,
            quit: false,
            log_events: false,
            open_reqs: Vec::new(),
            autoclose_ms: 0,
            accent_raw: 0,
            settle_warned: false,
            verify_dirty: false,
            frames_rendered: 0,
            close_at: 0,
            drects: Vec::new(),
            dmg: Vec::new(),
            cvs: Vec::new(),
            icon_hook: None,
            uiinput: UiInput::default(),
            sc: Default::default(),
            ucur: Default::default(),
            dr: Default::default(),
            dnd: Default::default(),
            tasks: plat::task::Executor::ui(),
        };
        for (font, raster) in style.font_raster().iter().enumerate() {
            ui.text.set_font_raster(font as u32, *raster);
        }
        ui.uiw_init();
        ui
    }

    // ------------------------------------------------------------------ queries

    /// ui_fb: the live window's framebuffer
    pub fn fb(&self) -> &Surface {
        &self.wp.rt.fb
    }

    /// ui_client_size: client area in DIP
    pub fn client_size(&self) -> (i32, i32) {
        (self.px2dip(self.wp.rt.fb.w), self.px2dip(self.wp.rt.fb.h))
    }

    /// g_anim_off: animations off (--anim off, the settings)  // pn-core
    pub fn anim_off(&self) -> bool {
        self.anim_off
    }
    pub fn set_anim_off(&mut self, off: bool) {
        self.anim_off = off;
    }

    /// ui_headless: running without a window (--shot / --script)
    pub fn headless(&self) -> bool {
        self.headless
    }

    /// ui_invalidate: request a new frame (state changed outside the callbacks)
    pub fn invalidate(&mut self) {
        self.need = true;
    }

    /// ui_quit: leave the main loop after this frame
    pub fn quit(&mut self) {
        self.quit = true;
    }

    /// ui_set_title(s): change the window title (kept for title() / title bars; 255 bytes)
    pub fn set_title(&mut self, s: &str) {
        let mut n = s.len().min(255);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.title = String::from(&s[..n]);
        if !self.headless {
            plat::set_title(&self.title);
        }
    }

    /// ui_title: the window title (the AppDesc title until set_title)
    pub fn title(&self) -> &str {
        if self.title.is_empty() { self.desc.title } else { &self.title }
    }

    /// the framebuffer as the platform presents it
    pub(crate) fn frame_of(sf: &Surface) -> plat::Frame<'_> {
        plat::Frame { px: &sf.px, w: sf.w, h: sf.h, stride: sf.stride }
    }

    /// png_save(path, sf): write a surface as a PNG file -> written (--shot and the scripts'
    /// shots: trusted, outside the app's permissions)
    pub(crate) fn save_png(sf: &Surface, path: &str) -> bool {
        let png = img_png::encode(&sf.px, sf.w as usize, sf.h as usize, sf.stride as usize, false);
        super::perm::trusted(|| plat::write_file(path, &png))
    }

    // call_cmd(id, arg) -> App::command
    fn call_cmd(&mut self, app: &mut dyn App, id: u32, arg: u32) -> bool {
        app.command(self, id, arg)
    }

    // ------------------------------------------------------------------ events

    /// ui_process_events: apply queued platform events; run passes for edges.  Every
    /// event belongs to a window (Event::win); input goes to the window uiw_input_target
    /// picks, which is made live for input_event.
    pub(crate) fn process_events(&mut self, app: &mut dyn App) {
        self.process_events_deferred(app, true);
    }

    // Browser batches retain every input edge and its app passes, but rasterize only
    // after the batch. Routing/capture changes must take effect before the next edge.
    pub(crate) fn process_events_deferred(&mut self, app: &mut dyn App, render: bool) {
        let mut text_pending = false;
        let mut text_win: i32 = -1;
        while let Some(mut ev) = plat::evq_pop() {
            if self.hr_on() {
                self.hr_popped(&ev); // (hotreload.rs) replayed or recorded
            }
            if !self.headless {
                let w = self.uiw_from_pw(ev.win as i32);
                if w < 0 {
                    ev_dropped(&ev);
                    continue; // a window the UI does not know (any more)
                }
                ev.win = w as u8;
            }
            if self.log_events {
                log_event(&ev);
            }
            let win = ev.win as u32;
            // lifecycle: which windows have the keyboard (UIC_ACTIVE after the batch)
            match ev.typ {
                plat::E_FOCUS_IN => self.app_focus |= 1 << (win & 31),
                plat::E_FOCUS_OUT => self.app_focus &= !(1 << (win & 31)),
                _ => {}
            }
            match ev.typ {
                plat::E_POWER => {
                    plat::log_info("app", if ev.data != 0 { "the system goes to sleep" } else { "the system woke up" });
                    self.call_cmd(app, UIC_SUSPEND, (ev.data != 0) as u32);
                    self.run_passes(app); // an edge: its own frame
                    self.need_render = true;
                    continue;
                }
                plat::E_SESSION_END => {
                    plat::log_info("app", "the session ends");
                    self.call_cmd(app, UIC_SESSION_END, 0);
                    self.quit = true; // UIC_EXIT follows
                    self.run_passes(app);
                    self.need_render = true;
                    continue;
                }
                plat::E_SETTINGS => {
                    self.settings_changed(app, ev.data);
                    self.run_passes(app);
                    self.need_render = true;
                    continue;
                }
                plat::E_CLOSE => {
                    if win != 0 {
                        self.uiw_close_request(win); // App::command(UIC_WINCLOSE, id)
                    } else if !self.call_cmd(app, UIC_CLOSE, 0) {
                        self.quit = true; // the app keeps the window open when handled
                    } else {
                        // kept open: what the app opened ("save changes?") shows in the next
                        // frame, not only after the next input or timer
                        self.need = true;
                    }
                    continue;
                }
                plat::E_RESIZE => {
                    self.uiw_resize(app, win, ev.x, ev.y);
                    continue;
                }
                plat::E_DPI => {
                    self.uiw_dpi(win, ev.data as i32);
                    continue;
                }
                plat::E_PAINT => continue,
                plat::E_WAKE => {
                    self.tasks_wake(); // wake-ups of tasks from other threads (tasks.rs)
                    self.cmd_run(UIC_WAKE, 0); // plat::wake() from another thread
                    self.open_poll();
                    self.tray_poll();
                    self.screen_poll(); // global shortcuts, picks, pictures (screen.rs)
                    self.a11y_poll(); // actions of screen readers (a11y.rs)
                    self.spell_poll(); // suggestions made on a worker (edit_spell.rs)
                    self.need = true;
                    continue;
                }
                plat::E_FOCUS_IN => {
                    self.uiw_focus_in(win);
                    continue;
                }
                plat::E_WINPOS => {
                    self.uiw_winpos(win, ev.x, ev.y);
                    continue;
                }
                plat::E_DROP => {
                    // (K1) the dropped content: onto a drop target (DS_DROP), else UIC_DROP
                    if !self.drop_receive(&ev) {
                        let id = self.uiw_rec_id(win);
                        self.cmd_run(UIC_DROP, id);
                    }
                    self.run_passes(app);
                    self.need_render = true;
                    continue;
                }
                plat::E_DRAG => {
                    self.dnd_hover(&ev); // a drag over the window: the targets highlight
                    self.run_passes(app);
                    self.need_render = true;
                    continue;
                }
                plat::E_DRAG_END => {
                    self.dnd_out_end(ev.data); // the platform finished the app's drag
                    self.run_passes(app);
                    self.need_render = true;
                    continue;
                }
                _ => {}
            }
            // (inspect.rs) the inspector's hot keys and its pick take their events first
            if self.m.ins.armed {
                let r = self.ins_event(&ev);
                if r != 0 {
                    if text_pending {
                        text_pending = false;
                        self.run_passes(app);
                    }
                    if r == 1 {
                        self.run_passes(app);
                    }
                    self.need_render = true;
                    continue;
                }
            }
            let t = self.uiw_input_target(&ev);
            if t < 0 {
                ev_dropped(&ev);
                continue; // a window that is gone
            }
            // a run of text ends before any other input (and before text for another window)
            if text_pending && (ev.typ != plat::E_TEXT || t != text_win) {
                text_pending = false;
                self.run_passes(app);
                self.need_render = true;
            }
            self.uiw_use(t as u32);
            if self.hr_on() {
                self.hr_input(&ev, t as u32); // (hotreload.rs) recorded or checked
            }
            match self.input_event(&ev) {
                2 => {
                    text_pending = true;
                    text_win = t;
                }
                0 => {}
                _ => {
                    self.run_passes(app); // an edge gets its own frame
                    self.need_render = true;
                }
            }
        }
        if text_pending {
            self.run_passes(app);
            self.need_render = true;
        }
        // the app as a whole got or lost the keyboard (focus moving between its windows
        // within one batch does not count; hot reload: recorded, not while replaying)
        let active = self.app_focus != 0;
        if active != self.app_active && self.hr_active(active) {
            self.app_active = active;
            if !active {
                // (§6) like the system's menus: the popups of every window close (they may
                // stand outside the windows, over other programs)
                self.uiw_popups_dismiss(WIN_MAX as u32);
            }
            self.call_cmd(app, UIC_ACTIVE, active as u32);
            self.run_passes(app);
            self.need_render = true;
        }
        if self.uiw_moved_passes() {
            self.run_passes(app);
            self.need_render = true;
        }
        if self.hr_on() {
            self.hr_batch_end(); // (hotreload.rs) the batch's records written
        }
        if render && self.need_render {
            self.need_render = false;
            // headless: render only to verify
            if !self.headless || self.verify_dirty {
                self.render();
                self.present();
            }
        }
        self.uiw_use(0);
    }

    // E_SETTINGS: follow the system's theme and format, then tell the app (UIC_SETTINGS)
    fn settings_changed(&mut self, app: &mut dyn App, bits: u32) {
        plat::log!(plat::LOG_INFO, "app", "system settings changed ({bits:#x})");
        if bits & (plat::SET_THEME | plat::SET_CONTRAST | plat::SET_FONTS | plat::SET_OTHER) != 0 {
            let before = (self.theme_dark, self.high_contrast);
            let s = if self.headless { plat::sys_settings_env() } else { plat::sys_settings() };
            if bits & plat::SET_THEME != 0 && self.theme_sys && s.dark.is_none() {
                self.theme_set(2); // no answer from the settings: plat::theme
            }
            self.sys_apply(s);
            if before != (self.theme_dark, self.high_contrast) {
                self.call_cmd(app, UIC_THEME, 0);
            }
        }
        if bits & plat::SET_LOCALE != 0 && !self.headless {
            crate::i18n::use_system();
            self.set_decimal_sep(crate::i18n::locale().decimal() as u32);
        }
        self.call_cmd(app, UIC_SETTINGS, bits);
        self.dirty_force_full();
        self.need = true;
    }

    /// ui_refresh: process pending events and redraw now (live resize callback)
    pub(crate) fn refresh(&mut self, app: &mut dyn App) {
        self.clock_update();
        self.process_events(app);
        self.run_passes(app);
        self.render();
        self.present();
    }

    /// ui_resize(w_px, h_px): a new framebuffer size of the live window
    pub(crate) fn resize(&mut self, app: &mut dyn App, w: i32, h: i32) {
        if w <= 0 || h <= 0 || (w == self.wp.rt.fb.w && h == self.wp.rt.fb.h) {
            return;
        }
        self.wp.rt.fb.resize(w, h);
        self.dirty_force_full();
        self.need = true;
        if self.headless {
            self.run_passes(app);
        }
    }

    // ------------------------------------------------------------------ passes

    /// ui_run_passes: App::frame until the layout settles, then deliver commands.  A pass
    /// starts in the main window; window_begin / window_end inside App::frame build the
    /// other windows in the same pass.
    pub(crate) fn run_passes(&mut self, app: &mut dyn App) {
        let (mut passes, mut build_us) = (0u32, 0u64); // (inspect.rs) statistics
        let mut tasks_ran = self.settle(app, &mut passes, &mut build_us);
        // typed text beyond a pass's queue (ctx.rs): part by part, each with passes of its own
        while self.text_refill() {
            tasks_ran = self.settle(app, &mut passes, &mut build_us);
        }
        self.dnd_after_passes(); // a DS_START without drag_set is no drag
        self.focus_req_expire();
        self.need = tasks_ran; // tasks ran in the last round: the frame did not see it yet
        self.a11y_frame(); // the accessibility tree for screen readers (a11y.rs)
        if self.m.ins.on {
            self.ins_passes(passes, build_us);
        } else if self.m.ins.gone {
            self.ins_gone(); // the inspector closed: its window is freed (inspect.rs)
        }
    }

    // settle -> async tasks ran in the last round: passes until the layout settles, the
    // queued commands, at most three rounds
    fn settle(&mut self, app: &mut dyn App, passes: &mut u32, build_us: &mut u64) -> bool {
        let mut rounds = 3;
        let mut tasks_ran;
        loop {
            let mut pass = 0;
            let mut keys_fwd = false; // (U3) keys handed on for the next pass
            loop {
                self.uiw_use(0);
                self.frame_begin();
                let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
                self.uiw_pass_begin(w, h);
                let csd = self.csd_begin();
                let t0 = if self.m.ins.on { plat::time_us() } else { 0 };
                app.frame(self);
                if t0 != 0 {
                    *build_us += plat::time_us().saturating_sub(t0);
                }
                self.uiw_unwind(); // windows the app did not end
                self.ids_unwind(); // id scopes it did not close
                self.perm_dialog(); // an open permission question (perm.rs)
                // frameless windows (own or library title bar): resize grips on top
                if csd || self.desc.flags & AF_CUSTOM_TITLEBAR != 0 {
                    self.edge_grips();
                }
                if csd {
                    self.box_end();
                    self.box_end();
                }
                self.uiw_pass_end();
                if self.m.ins.on {
                    // the inspector's tool window, after the main window's pass (with the
                    // menus and tool tips of its end): inspect.rs
                    self.inspect_frame();
                }
                let mut again = self.frame_end();
                if pass == 0 {
                    self.uiw_clear_edges();
                    // (U3) keys a tool window borrowed but did not use: its owner's input
                    // for pass 2
                    keys_fwd = self.uiw_fwd_inject();
                    again |= keys_fwd;
                } else if keys_fwd {
                    keys_fwd = false;
                    self.uiw_clear_edges(); // the handed-on keys were seen once
                }
                pass += 1;
                *passes += 1;
                if !again {
                    break;
                }
                if pass >= MAX_PASSES {
                    if !self.settle_warned {
                        self.settle_warned = true;
                        self.warn("layout did not settle within 4 passes");
                    }
                    break;
                }
            }
            // windows not built in this frame disappear; sizes that follow the content
            let changed = self.uiw_settle();
            // deliver queued commands; if any ran, build the frame again
            let mut ran = false;
            while let Some((id, arg)) = self.cmd_take() {
                ran = true;
                // the inspector's window: its close button, Esc and drops are its own
                if arg == super::inspect::INSPECT_WIN && self.ins_cmd(id) {
                    continue;
                }
                let r = self.call_cmd(app, id, arg);
                self.uiw_cmd_done(id, arg, r); // UIC_WINCLOSE not handled: close it
                if !r && id == UIC_CLOSE {
                    self.quit = true;
                }
            }
            // async tasks woken meanwhile (tasks.rs): what they changed shows in the next pass
            tasks_ran = self.tasks_pump();
            if !(ran || changed || tasks_ran) {
                break;
            }
            rounds -= 1;
            if rounds == 0 {
                break;
            }
        }
        tasks_ran
    }

    // csd_begin -> true when the library draws the title bar for the app: the window has no
    // server-side decorations (WST_CSD) and the app has no AF_CUSTOM_TITLEBAR.  Then the
    // frame is a column: title bar, the app's area (two boxes to close after App::frame).
    // jfgui_CSD=1 forces it (tests, headless shots).
    fn csd_begin(&mut self) -> bool {
        if self.desc.flags & AF_CUSTOM_TITLEBAR != 0 {
            return false;
        }
        let forced = plat::getenv("jfgui_CSD").is_some_and(|v| v.as_bytes().first() == Some(&b'1'));
        if !forced && (self.headless || plat::window_state() & plat::WST_CSD == 0) {
            return false;
        }
        self.next_size(-1, -1);
        self.box_begin(0xFFFF_7420, BOX_V | BOX_FIXED, 0);
        let t = String::from(self.title());
        self.titlebar_begin(&t, 0);
        self.titlebar_end();
        self.next_size(-1, -1);
        self.box_begin(0xFFFF_7421, BOX_V | BOX_FIXED, 0);
        true
    }

    // ------------------------------------------------------------------ rendering

    /// ui_render: replay the recorded commands of every shown window into its framebuffer,
    /// only inside the dirty rectangles (everything on the first frame / after a resize),
    /// and present each window right away
    pub(crate) fn render(&mut self) {
        if !self.uiw_any_used() {
            self.render_live(); // the main window only
            self.uiw_popups_render(0); // (§6) its popups outside the window
            self.present();
            return;
        }
        for i in 0..WIN_MAX as u32 {
            if !self.uiw_drawable(i) {
                self.uiw_popups_drop(i); // (U3) not drawn: no popup windows
                continue;
            }
            self.uiw_use(i);
            if self.uiw_take_newshow(i) {
                self.dirty_force_full(); // a window shown again: everything
            }
            self.render_live();
            self.uiw_popups_render(i); // (U3) popups outside the window
            self.present();
        }
        self.uiw_use(0);
        self.uiw_sync_show(); // new windows appear with their content
    }

    // render_live: ui_render for the live window
    fn render_live(&mut self) {
        self.wp.rt.present_n = 0;
        if self.wp.rt.fb.px.is_empty() {
            return; // no framebuffer (allocation failed)
        }
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        self.cmd_mirror(); // right-to-left: the recorded commands across the window
        let mut n = self.dirty_compute(w, h);
        let ins = self.m.ins.on;
        if ins {
            n = self.ins_render_begin(n); // the inspector's overlay (inspect.rs)
        }
        self.wp.rt.present_n = self.scopy_present(n); // + the cells a scroll copy moved (scrollcopy.rs)
        if self.ui_debug {
            plat::write_stderr(alloc::format!("render: {} dirty rects (-1 = full)\n", n).as_bytes());
        }
        if self.wp.rt.present_n == 0 {
            return; // nothing changed
        }
        self.dc.begin(&self.wp.rt.fb);
        // the text font the passes left set stays set (the replay sets each text's font,
        // custom draws their own; what stays must not depend on the dirty part or on threads)
        let font = self.text.get_font();
        if self.render_par(n) {
            n = 0; // drawn on several threads (render_par.rs)
        }
        if n < 0 {
            self.render_all_to(false);
        } else {
            for i in 0..n as usize {
                let r = self.drects[i];
                if self.ui_debug {
                    plat::write_stderr(alloc::format!("  dirty {},{} {}x{}\n", r[0], r[1], r[2], r[3]).as_bytes());
                }
                self.dc.clip_push(r[0], r[1], r[2], r[3]);
                self.render_all_to(false);
                self.dc.clip_pop();
            }
        }
        self.dc.end();
        self.scopy_done(); // the frame's cost (scrollcopy.rs)
        if ins {
            self.ins_render_end(); // the inspector's overlay on top (inspect.rs)
        }
        self.frames_rendered = self.frames_rendered.wrapping_add(1);
        if self.verify_dirty {
            self.verify_dirty();
        }
        self.text.set_font(font.0, font.1);
    }

    /// ui_present: show what render changed in the live window
    pub(crate) fn present(&mut self) {
        let n = self.wp.rt.present_n;
        if n == 0 {
            return;
        }
        self.uiw_present(n);
        self.wp.rt.present_n = 0;
    }

    // verify_dirty: render the whole frame into a shadow surface and compare
    // (--verify-dirty): any difference means a command's box was too small (auto.rs' bench
    // calls it after the timed render)
    pub(crate) fn verify_dirty(&mut self) {
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        if self.wp.rt.shadow.w != w || self.wp.rt.shadow.h != h {
            self.wp.rt.shadow = Surface::new(w, h, FMT_XRGB);
        }
        self.dc.begin(&self.wp.rt.shadow);
        self.render_all_to(true);
        self.dc.end();
        if self.m.ins.on {
            self.ins_verify(true); // the inspector's overlay belongs to the frame (inspect.rs)
        }
        // compare the RGB of every pixel
        let (mut n, mut x0, mut y0, mut x1, mut y1) = (0u32, i32::MAX, i32::MAX, -1i32, -1i32);
        {
            let (a, b) = (&self.wp.rt.shadow, &self.wp.rt.fb);
            for y in 0..a.h {
                let ra = y as usize * a.stride as usize;
                let rb = y as usize * b.stride as usize;
                for x in 0..a.w {
                    let i = x as usize * 4;
                    if a.px[ra + i..ra + i + 3] != b.px[rb + i..rb + i + 3] {
                        n += 1;
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x);
                        y1 = y; // rows run downwards: last = max
                    }
                }
            }
        }
        if n == 0 {
            return;
        }
        self.warn(&alloc::format!("dirty regions missed {} pixels", n));
        let s = alloc::format!("  in px {},{} .. {},{} (window {})\n", x0, y0, x1, y1, self.uiw_cur);
        plat::write_stderr(s.as_bytes());
        // repair the frame so later frames stay comparable
        self.dc.begin(&self.wp.rt.fb);
        self.render_all_to(false);
        self.dc.end();
        if self.m.ins.on {
            self.ins_verify(false);
        }
        self.wp.rt.present_n = -1;
    }

    // ------------------------------------------------------------------ clock

    // clock_update: now_ms from the platform clock (window mode)
    fn clock_update(&mut self) {
        let ms = (plat::time_us().wrapping_sub(self.t0_us) / 1000) as u32;
        self.set_time(ms);
    }

    // ui_timer_due_peek -> earliest timer deadline (ms; "now" while one has expired) or -1
    fn timer_due_peek(&self) -> u32 {
        match self.timer_scan() {
            (Some(_), _) => self.now_ms,
            (None, next) => next,
        }
    }
}

// ev_dropped: an event that goes to no window; a commit's text leaves the store with it (the
// store keeps a few: plat::ime_commit_set)
fn ev_dropped(ev: &Event) {
    if ev.typ == plat::E_IME_COMMIT {
        let _ = plat::ime_commit_take(ev.data);
    }
}

// log_event: print an event record
fn log_event(ev: &Event) {
    let s = alloc::format!(
        "event type={} btn={} mods={:x} x={} y={} data={:x} win={}\n",
        ev.typ, ev.btn, ev.mods, ev.x, ev.y, ev.data, ev.win
    );
    plat::write_stdout(s.as_bytes());
}

/// jfgui_main of an app: the runtime for one AppDesc + App -> exit code
fn prepare(desc: &AppDesc, style: &'static dyn Style) -> (Ui, i32, i32) {
    // the image codecs: file access through plat, the core formats registered (the asm
    // links PNG / JPEG / BMP / GIF statically)
    img_core::io::set_file_io(img_core::io::FileIo { read: plat::read_file, write: plat::write_file });
    img_all::use_core_codecs();
    img_svg::register(); // (svg) .svg / .svgz through img_core::io::load / load_ex
    plat::set_print_deflate(Some(img_core::deflate::zlib_deflate)); // PDFs of plat::print as the asm's
    img_core::set_parallel(icc_par); // the codecs' parallel parts (YUV bands, ICC grids) on the pool
    let args = Args(plat::args());
    let mut ui = Ui::new(desc, style);
    // the decimal sign of number boxes: the user's format (headless: German)
    ui.set_decimal_sep(crate::i18n::locale().decimal() as u32);
    // ---- options
    let (mut w, mut h) = (desc.w as i32, desc.h as i32);
    if let Some(v) = args.value("--size") {
        // arg_size: "WxH", both > 0
        let (a, r) = parse_int(v.as_bytes());
        if !r.is_empty() && r[0] | 32 == b'x' {
            let (b, _) = parse_int(&r[1..]);
            if a > 0 && b > 0 {
                w = a;
                h = b;
            }
        }
    }
    ui.wp.rt.dpi = scale_to_dpi(args.int("--scale", 100));
    ui.log_events = args.flag("--log-events");
    ui.verify_dirty = args.flag("--verify-dirty");
    ui.autoclose_ms = args.int("--auto-close", 0) as u32;
    ui.inspect_args(args.flag("--inspect")); // the live inspector (inspect.rs)
    if let Some(a) = args.value("--anim") {
        ui.anim_off = a == "off";
        ui.anim_forced = true;
    }
    // high contrast and text scale: --contrast high|normal, --text-scale PERCENT
    match args.value("--contrast") {
        Some("high") => ui.contrast_mode = 1,
        Some("normal") => ui.contrast_mode = 2,
        _ => {}
    }
    if let Some(v) = args.value("--text-scale") {
        if let Ok(p) = v.parse::<u32>() {
            ui.text_scale = p.clamp(50, 300);
            ui.text_scale_forced = true;
        }
    }
    // right-to-left: --rtl / --ltr, else the UI language
    if args.flag("--rtl") {
        ui.set_rtl(Some(true));
    } else if args.flag("--ltr") {
        ui.set_rtl(Some(false));
    }
    // theme: --theme, else the app flags, else the system setting
    let mut mode = 2;
    if desc.flags & AF_DARK != 0 {
        mode = 1;
    }
    if desc.flags & AF_LIGHT != 0 {
        mode = 0;
    }
    if let Some(t) = args.value("--theme") {
        if t == "dark" {
            mode = 1;
        }
        if t == "light" {
            mode = 0;
        }
    }
    ui.accent_raw = desc.accent & 0xFF_FFFF;
    if let Some(a) = args.value("--accent") {
        ui.accent_raw = parse_hex(a.as_bytes()).0 & 0xFF_FFFF;
    }
    let acc = ui.accent_raw;
    ui.accent_set(acc);
    ui.accent_sys = acc == 0; // no accent of the app: the system's
    ui.theme_set(mode);
    (ui, w, h)
}

// --theme-file PATH: an app theme from the command line, after App::init (it wins over the
// app's own); an error is printed and the app's theme stays
fn cli_theme(ui: &mut Ui) {
    let args = Args(plat::args());
    if let Some(f) = args.value("--theme-file") {
        if let Err(e) = super::perm::trusted(|| ui.load_theme(f)) {
            plat::write_stderr(alloc::format!("error: --theme-file: {}\n", e).as_bytes());
        }
    }
}

pub fn run(desc: &AppDesc, app: &mut dyn App, style: &'static dyn Style) -> i32 {
    let (mut ui, w, h) = prepare(desc, style);
    let ui = &mut ui;
    ui.hr_init(); // hot reload: jfgui_HOTRELOAD (hotreload.rs; development builds)
    let args = Args(plat::args());
    let shot = args.value("--shot");
    let script = args.value("--script");
    let agent_mode = if args.flag("--agent-json") { Some(agent_core::Protocol::Json) }
        else if args.flag("--agent-mcp") { Some(agent_core::Protocol::Mcp) } else { None };
    if shot.is_none() && script.is_none() && agent_mode.is_none() {
        return window_main(ui, app, w, h);
    }
    // ---- headless run (virtual clock starting at 0)
    ui.headless = true;
    ui.sys_apply(plat::sys_settings_env()); // only jfgui_* overrides: shots match everywhere
    // missing characters from system fonts: only on request, so shots stay the same everywhere
    ui.text.set_system_fallback(plat::env("jfgui_FONT_FALLBACK").is_some_and(|v| v.starts_with('1')));
    let (pw, ph) = (ui.dp(w), ui.dp(h));
    ui.wp.rt.fb = Surface::new(pw, ph, FMT_XRGB);
    if ui.wp.rt.fb.px.is_empty() && pw * ph != 0 {
        plat::write_stderr(b"error: out of memory\n");
        return 2;
    }
    ui.set_time(0);
    if !app.init(ui) {
        plat::write_stderr(b"error: app initialisation failed\n");
        return 2;
    }
    cli_theme(ui);
    ui.run_passes(app);
    // hot reload: the session's log replayed first (hotreload.rs)
    if !ui.hr_start(app) {
        return super::hotreload::HR_EXIT_DIVERGED;
    }
    if let Some(protocol) = agent_mode {
        let r = super::agent::serve(ui, app, protocol);
        ui.hr_end();
        return r;
    }
    if let Some(s) = script {
        let r = ui.script_run(app, s);
        ui.hr_end();
        return r;
    }
    let shot = shot.unwrap_or("");
    ui.render();
    if !Ui::save_png(&ui.wp.rt.fb, shot) {
        plat::write_stderr(alloc::format!("error: could not write {}\n", shot).as_bytes());
        return 1;
    }
    if args.flag("--dump") {
        ui.dump(None);
    }
    ui.hr_end();
    plat::write_stdout(alloc::format!("wrote {}\n", shot).as_bytes());
    0
}

/// the interactive window loop (ui_main.s .am_window): the platform's window-indexed API
/// (plat::window_create, wait_refresh, win_present, the hit hook) on every OS; the live
/// redraw inside the Win32 modal size / move loop goes through plat::wait_refresh (the
/// Linux backends have no modal loop)
fn window_main(ui: &mut Ui, app: &mut dyn App, w: i32, h: i32) -> i32 {
    // AF_SINGLE_INSTANCE: a running instance takes this start's command line (UIC_OPEN there)
    if ui.desc.flags & AF_SINGLE_INSTANCE != 0 && plat::instance::claim(&plat::vfs::app_id()) == plat::instance::Claim::Forwarded {
        return 0;
    }
    plat::win_hit_hook(Some(super::win::uiw_hit_pw)); // non-client hit tests ask the UI
    plat::win_move_hook(Some(super::win::uiw_moving_pw)); // user moves snap (window_snap)
    plat::drag_hook(Some(super::win::uiw_drag_pw)); // so do drags of other programs (dnd.rs)
    plat::set_min_size(ui.desc.min_w as i32, ui.desc.min_h as i32);
    // AF_SAVE_STATE / jfgui_STATE: the placement of the last run (winstate.rs); --size wins
    let state = super::winstate::state_file(ui.desc.flags);
    let saved = state.as_deref().map(base::Ini::load);
    let mut main = saved.as_ref().and_then(super::winstate::main_saved);
    if Args(plat::args()).value("--size").is_some() {
        main = None;
    }
    let (cw, ch) = main.map_or((w, h), |m| (m.w.max(ui.desc.min_w as i32), m.h.max(ui.desc.min_h as i32)));
    let (x, y) = match main {
        Some(m) if m.pos => (m.x, m.y),
        _ => (plat::PW_DEFAULT, plat::PW_DEFAULT),
    };
    if !plat::window_create_at(ui.desc.title, cw, ch, ui.desc.flags, x, y, main.is_some_and(|m| m.max)) {
        plat::write_stderr(b"error: no window could be created (use --shot FILE.png for a headless run)\n");
        return 2;
    }
    if main.is_some_and(|m| m.pos && !m.max) {
        main_onscreen(); // a monitor of the last run may be gone
    }
    ui.win_hooks_set(true); // ui_win_hooks_set(uiw_plat_hooks): window_begin windows are OS windows
    log_start(ui);
    ui.sys_apply(plat::sys_settings()); // theme, accent, contrast, motion, text scale, timing
    // missing characters from system fonts (jfgui_FONT_FALLBACK=0 turns it off)
    ui.text.set_system_fallback(!plat::env("jfgui_FONT_FALLBACK").is_some_and(|v| v.starts_with('0')));
    ui.wp.rt.dpi = plat::dpi();
    let (pw, ph) = plat::window_size();
    ui.wp.rt.fb = Surface::new(pw, ph, FMT_XRGB);
    ui.t0_us = plat::time_us();
    ui.clock_update();
    super::perm::ask_here(true); // permission questions: a dialog on this thread (perm.rs)
    if !app.init(ui) {
        plat::write_stderr(b"error: app initialisation failed\n");
        return 2;
    }
    cli_theme(ui);
    ui.need = true;
    if Args(plat::args()).flag("--agent") { ui.set_agent_enabled(true); }
    ui.close_at = ui.autoclose_ms;
    // the main window's normal placement until the platform reports one; the tool windows of
    // the last run wait for window_begin
    if let Some(m) = main {
        ui.m.win.norm = super::winstate::MainState { max: false, ..m };
    }
    ui.ws_track();
    if let Some(ini) = &saved {
        ui.ws_windows(ini);
    }
    // hot reload: the session's log replayed before the first wait (hotreload.rs); diverged:
    // the watcher starts the app again afresh
    if !ui.hr_start(app) {
        ui.call_cmd(app, UIC_EXIT, 0);
        ui.set_agent_enabled(false);
        super::perm::ask_here(false);
        return super::hotreload::HR_EXIT_DIVERGED;
    }
    loop {
        if ui.quit {
            break;
        }
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        ui.agent_poll(app);
        if ui.need {
            ui.need = false;
            ui.run_passes(app);
            // a command of this frame may have ended the app (UIC_CLOSE, quit): leave
            // before waiting for the next event
            if ui.quit {
                break;
            }
            ui.render();
            ui.present();
        }
        // deadline in ms since start: animations (16 ms), timers, auto close
        let mut dl = if ui.agent_enabled() { ui.now_ms.wrapping_add(30) } else { u32::MAX };
        if ui.animating {
            dl = ui.now_ms.wrapping_add(16);
        }
        let t = ui.timer_due_peek();
        if t < dl {
            dl = t;
        }
        if ui.close_at != 0 && ui.close_at < dl {
            dl = ui.close_at;
        }
        if ui.hr_on() && ui.hr_deadline().wrapping_sub(ui.now_ms) < dl.wrapping_sub(ui.now_ms) {
            dl = ui.hr_deadline(); // hot reload: the watcher's files (hotreload.rs)
        }
        // ms since start -> absolute us; tasks: their timers (the task clock is time_us here),
        // no wait while some are ready
        let mut deadline = if dl == u32::MAX { u64::MAX } else { dl as u64 * 1000 + ui.t0_us };
        if let Some(t) = ui.tasks_deadline() {
            deadline = deadline.min(t.max(1));
        }
        if ui.tasks_ready() {
            deadline = 0;
        }
        // the live redraw inside the Win32 modal size / move loop (ui_refresh)
        plat::wait_refresh(deadline, &mut || ui.refresh(app));
        ui.clock_update();
        if ui.close_at != 0 && ui.close_at <= ui.now_ms {
            break;
        }
        if ui.animating || ui.tasks_ready() {
            ui.need = true;
        }
        if ui.hr_on() {
            ui.hr_poll(); // hot reload: "quit", a failed build (hotreload.rs)
        }
        ui.tick_timers();
        ui.process_events(app);
    }
    // the placement for the next run (winstate.rs), then every way out of the window loop
    // (close, quit, --auto-close) tells the app (U3)
    if let Some(p) = &state {
        let mut ini = base::Ini::load(p);
        ui.state_save(&mut ini);
        if !ini.save(p) {
            plat::log_warn("app", &alloc::format!("could not write {}", p));
        }
    }
    ui.hr_end(); // hot reload: the log's end, the carried state (hotreload.rs)
    ui.call_cmd(app, UIC_EXIT, 0);
    ui.set_agent_enabled(false);
    super::perm::ask_here(false); // threads still waiting for an answer are refused
    plat::log_info("app", "exit");
    0
}

// the main window just created at a saved position: moved into the work area of the nearest
// monitor when it would be (almost) off every screen
fn main_onscreen() {
    let (Some(r), Some(area)) = (plat::win_get_rect(0), plat::win_work_area(0)) else { return };
    let (x, y) = super::winstate::fit_area(r[0], r[1], r[2], r[3], area);
    if (x, y) != (r[0], r[1]) {
        plat::win_set_rect(0, x, y, plat::PW_KEEP, plat::PW_KEEP);
    }
}

// window mode: the log file (AF_LOG_FILE, jfgui_LOG_FILE = a path, 1 or 0) and the start line
fn log_start(ui: &Ui) {
    let env = plat::env("jfgui_LOG_FILE");
    let path = match env.as_deref() {
        Some("0") | Some("") => None,
        Some("1") => Some(alloc::format!("logs://{}.log", plat::vfs::app_id())),
        Some(p) => Some(String::from(p)),
        None if ui.desc.flags & AF_LOG_FILE != 0 => Some(alloc::format!("logs://{}.log", plat::vfs::app_id())),
        None => None,
    };
    if let Some(p) = path {
        if !plat::log_file(Some(&p), 1 << 20, 3) {
            plat::log_warn("app", &alloc::format!("the log file {p} cannot be written"));
        }
    }
    plat::log!(plat::LOG_INFO, "app", "start {} ({}, {})", ui.desc.title, crate::i18n::locale_tag(), crate::i18n::language_code());
}

/// plat::parallel_for for img_core (which does not depend on plat)
fn icc_par(n: usize, f: &(dyn Fn(usize) + Sync)) {
    let _ = plat::parallel_for(n, &|i, _| f(i));
}
