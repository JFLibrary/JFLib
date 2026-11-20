// bench -- (G7b part 2) path (a) of the game window measured: the gpu_view over the whole window, lib/ui's HUD over
// it (docs/GAMECORE.md "Spielfenster")
//
//   gamedemo --bench view [--present fifo|mailbox|immediate] [--fullscreen] [--limit HZ] [--n N] [--secs S]
//            [--steps hud,veil,readback]
//
// One window of 1280 x 800 (no status bar: the view covers it, as in game 1), the camera turning by itself (a
// steady mouse look, 0.6 rad/s), a frame per pass of the window loop (ui.timer(1, 1), as game 1 and the demo; with
// --limit a new frame of the view at most HZ times a second).  Steps, each with N presses (probe.rs: input to the
// screen) and S seconds without (the frames on the screen):
//   none      no HUD over the view
//   veil      the HUD over it, lib/ui composes it on the GPU (veils, the default since G7b part 2)
//   readback  the HUD over it the way before: those cells read every frame of the view back (Ui::gpu_test_veil)
// The present mode is the UI's (MAILBOX unless jfgui_GPU_PRESENT says otherwise; --present sets it).  The lines
// RESULT ... go to stdout, one per step and phase.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use game::{Pose, Vec3};
use ui::*;

use crate::probe::{Phase, Probe, Report};
use crate::{arg, has, hud, HudState, View, DESC, EYE, ID_VIEW, TIMER_FRAME};

/// the camera's turn while measuring (radians a second)
pub(crate) const TURN: f64 = 0.6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hud {
    None,
    Veil,
    Readback,
}

impl Hud {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Hud::None => "none",
            Hud::Veil => "veil",
            Hud::Readback => "readback",
        }
    }
}

/// the steps of a run: (HUD, phase)
pub(crate) fn steps(own: bool) -> Vec<(Hud, Phase)> {
    let list = arg("--steps").unwrap_or_else(|| String::from(if own { "none,hud" } else { "none,veil,readback" }));
    let mut v = Vec::new();
    for s in list.split(',') {
        let h = match s {
            "none" => Hud::None,
            "veil" | "hud" => Hud::Veil,
            "readback" => Hud::Readback,
            _ => continue,
        };
        v.push((h, Phase::Latency));
        v.push((h, Phase::Pacing));
    }
    v
}

/// the run's steps one after the other: settle, measure, report
pub(crate) struct Plan {
    pub(crate) probe: Rc<Probe>,
    steps: Vec<(Hud, Phase)>,
    at: usize,
    /// when the step began (0: not yet), when its measurement began
    since: u64,
    measuring: bool,
    n: usize,
    secs: f64,
    pub(crate) reports: Vec<Report>,
    pub(crate) label: String,
}

/// what a pass of the plan says
pub(crate) enum Next {
    /// warm up or settle: the HUD as given
    Wait(Hud),
    /// measuring with this HUD
    Run(Hud),
    Done,
}

impl Plan {
    pub(crate) fn new(own: bool, label: String) -> Plan {
        let n = arg("--n").and_then(|v| v.parse().ok()).unwrap_or(150);
        let secs = arg("--secs").and_then(|v| v.parse().ok()).unwrap_or(3.0);
        Plan { probe: Rc::new(Probe::start()), steps: steps(own), at: 0, since: 0, measuring: false, n, secs, reports: Vec::new(), label }
    }

    /// a pass: now (us), the presses the app has taken so far
    pub(crate) fn pass(&mut self, now: u64, presses: u32) -> Next {
        let Some(&(hud, phase)) = self.steps.get(self.at) else { return Next::Done };
        if self.since == 0 {
            self.since = now;
        }
        if !self.measuring {
            // (the first step: the device and the swapchain warm up; later ones: the HUD's change settles)
            let settle = if self.at == 0 { 2_500_000 } else { 600_000 };
            if now >= self.since + settle {
                self.measuring = true;
                self.probe.begin(phase, presses);
            }
            return Next::Wait(hud);
        }
        let done = match phase {
            Phase::Latency => self.probe.presses() >= self.n && now >= self.since + 300_000,
            Phase::Pacing => now >= self.since + (self.secs * 1e6) as u64,
        };
        if done {
            let name = format!("{} hud={} phase={}", self.label, hud.name(), if phase == Phase::Latency { "latency" } else { "pacing" });
            let r = self.probe.end(&name);
            plat::write_stdout(format!("{}\n", r.line()).as_bytes());
            self.reports.push(r);
            self.at += 1;
            self.since = now;
            self.measuring = false;
            // (the next step of the same HUD needs no settling)
            if self.steps.get(self.at).is_some_and(|s| s.0 == hud) {
                self.measuring = true;
                let (_, p) = self.steps[self.at];
                self.probe.begin(p, presses);
            }
        } else if phase == Phase::Latency && self.probe.presses() < self.n {
            // (the step's clock starts again while presses come: the last one gets 300 ms)
            self.since = now;
        }
        Next::Run(hud)
    }
}

struct App1 {
    view: Rc<RefCell<View>>,
    ver: u32,
    plan: Plan,
    last: u64,
    /// --limit: the time of the view's next frame (us), the period
    next: u64,
    period: u64,
    look_at: u64,
    fullscreen: bool,
    fps: (u64, u32, f64),
    started: bool,
}

impl App for App1 {
    fn frame(&mut self, ui: &mut Ui) {
        let (w, h) = (ui.fb().w, ui.fb().h);
        ui.next_rect_px(0, 0, w, h);
        let _ = ui.gpu_view(ID_VIEW, &self.view, self.ver);
        let now = plat::time_us();
        if !self.started {
            self.started = true;
            if self.fullscreen {
                ui.window_fullscreen(0, true);
            } else if let Some(a) = plat::win_work_area(0) {
                // (the same place every run: the top left of the work area)
                plat::win_set_rect(0, a[0] + 8, a[1] + 40, plat::PW_KEEP, plat::PW_KEEP);
            }
        }
        // the probe's point: the sky at the top's middle (no HUD there), on the screen
        let (cx, cy) = (w / 2, 12);
        if now >= self.look_at {
            self.look_at = now + 250_000;
            self.plan.probe.locate(0, cx, cy, true);
        }
        let dt = if self.last == 0 { 0.0 } else { now.saturating_sub(self.last) as f64 * 1e-6 };
        self.last = now;
        let (tick, presses) = {
            let mut guard = self.view.borrow_mut();
            let v = &mut *guard;
            // the keys whatever has the focus: the probe's presses (F9) are the game's action "marker"
            while let Some(k) = ui.key_event_take() {
                v.game.res.input.key(k.code, k.down);
                if k.down && !k.repeat && k.code == plat::KC_F9 {
                    v.presses += 1;
                    self.plan.probe.seen();
                }
            }
            v.cam.look(-TURN * dt, 0.0);
            v.game.res.yaw = v.cam.yaw;
            let n = v.game.clock.advance_to(now.saturating_mul(1000));
            for _ in 0..n {
                v.game.res.tick = v.game.res.input.tick();
                v.game.step();
            }
            v.game.res.input.end_frame();
            let alpha = v.game.alpha();
            let feet = v.game.world.get::<Pose>(v.game.res.walker).map_or(Vec3::ZERO, |p| p.interpolate(alpha).pos);
            v.cam.eye = feet + Vec3::new(0.0, EYE, 0.0);
            v.assets_frame();
            (v.game.world.tick(), v.presses)
        };
        let step = self.plan.pass(now, presses);
        let hud_on = match step {
            Next::Done => {
                self.view.borrow_mut().probe = None;
                let o = ui.gpu_output(0);
                let i = ui.gpu_view_info(0, ID_VIEW).unwrap_or_default();
                plat::write_stdout(format!("path (a): present {}, adapter {}, {} x {}, frames presented {}, view frames {}, read-backs {}, veils drawn {}, settled {}, held {}\n",
                                           o.present_mode, o.adapter, w, h, o.frames, i.frames, i.reads, o.veil_draws, o.veil_settles, i.held).as_bytes());
                ui.quit();
                return;
            }
            Next::Wait(hd) | Next::Run(hd) => {
                ui.gpu_test_veil(hd != crate::bench::Hud::Readback);
                hd != crate::bench::Hud::None
            }
        };
        self.fps.1 += 1;
        if now >= self.fps.0 + 500_000 {
            self.fps.2 = self.fps.1 as f64 * 1e6 / now.saturating_sub(self.fps.0).max(1) as f64;
            (self.fps.0, self.fps.1) = (now, 0);
        }
        if hud_on {
            hud(ui, [0, 0, w, h], &HudState { playing: true, tick, fps: self.fps.2, health: 100 - (tick / 60 % 100) as i32 });
        }
        // a new frame of the view: every pass, or at most every period (--limit)
        if self.period == 0 || now >= self.next {
            self.ver = self.ver.wrapping_add(1);
            self.next = if self.next == 0 || now > self.next + self.period { now + self.period } else { self.next + self.period };
        }
        ui.timer(TIMER_FRAME, 1);
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id == UIC_TIMER && arg == TIMER_FRAME {
            ui.invalidate();
            return true;
        }
        false
    }
}

/// path (a) measured (the module's header) -> the exit code
pub(crate) fn view() -> i32 {
    plat::setenv("jfgui_STATE", Some("0"));
    plat::setenv("jfgui_TEST_NOACTIVATE", Some("1"));
    if let Some(p) = arg("--present") {
        plat::setenv("jfgui_GPU_PRESENT", Some(&p));
    }
    let limit: f64 = arg("--limit").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let fullscreen = has("--fullscreen");
    let label = format!("path=a present={} window={} limit={}", arg("--present").unwrap_or_else(|| String::from("default")), if fullscreen { "full" } else { "1280x800" }, limit);
    let plan = Plan::new(false, label);
    let mut v = View::new(game::assets::Loading::default_for_target(), None);
    v.probe = Some(plan.probe.clone());
    // (no pointer lock in a measurement: the game plays without it, the probe's key counts)
    v.game.res.input.set_require_lock(false);
    let mut app = App1 {
        view: Rc::new(RefCell::new(v)),
        ver: 0,
        plan,
        last: 0,
        next: 0,
        period: if limit > 0.0 { (1e6 / limit) as u64 } else { 0 },
        look_at: 0,
        fullscreen,
        fps: (0, 0, 0.0),
        started: false,
    };
    let d = AppDesc { title: "Spiel-Demo: Messung", w: 1280, h: 800, ..DESC };
    let code = ui::run_window(&d, &mut app, &style::STYLE);
    app.plan.probe.stop();
    code
}
