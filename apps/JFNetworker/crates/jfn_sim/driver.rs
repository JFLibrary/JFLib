// driver -- runs the engine for a user interface without threads (Inline)
//
//   Inline::new(engine)                  starts paused
//   send(Cmd, wall_us) -> Result<Tick>   a command from the UI
//   tick(wall_us) -> Tick                call from a timer; in realtime the virtual clock follows
//                                        the wall clock times the speed
//   view() / view_time()                 the moment the view shows (simulation mode)
//
// Modes: Paused (nothing moves), Realtime { speed_pm } (1000 = real time, 250 = quarter speed),
// Simulation (only steps move: StepEvent, StepHop { mask } runs until the next frame whose
// protocol bits match, as Packet Tracer's "capture / forward").  Realtime runs at most `budget`
// events per tick; when it cannot keep up, the clock is re-anchored instead of catching up.
// In headless script runs the caller passes the script's virtual wall clock, so runs repeat.
//
// Simulation mode steps from one moment a frame is sent to the next: Step(Filter) for the
// frames of some protocols or devices.  StepBack(Filter) moves the view back along the
// recording to the previous such moment -- only the view: the devices keep their present
// state (their state at that time comes with keyframes later).  While the view stands in the
// past, Step walks forward through the recording, and at the present it runs the engine again;
// Live returns at once.  Commands that run the engine (realtime, RunFor, RunIdle, StepEvent)
// return to the present first.  Seek(t) puts the view at any moment of the recording (the
// playhead of a timeline, a line of the event list).

use crate::action::{ActErr, Action};
use crate::kernel::{Engine, Ns, RunOut};
use crate::trace::Filter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Paused,
    Realtime { speed_pm: u32 },
    Simulation,
}

#[derive(Clone, Debug)]
pub enum Cmd {
    Realtime { speed_pm: u32 },
    Pause,
    Simulation,
    StepEvent,
    StepHop { mask: u32 },
    /// to the next moment a frame the filter passes is sent: forward through the recording
    /// while the view stands in the past, else the engine runs to it
    Step(Filter),
    /// the view goes back to the previous moment a frame the filter passes was sent (the
    /// devices keep their present state)
    StepBack(Filter),
    /// the view shows the present again
    Live,
    RunFor(Ns),
    RunIdle,
    Apply(Action),
    /// a line typed on a device's console (Enter); output comes as Report::CliOut
    Cli { dev: u32, line: alloc::string::String },
    /// Ctrl+C (Linux) or Ctrl+Shift+6 (IOS) on a device's console
    Interrupt { dev: u32 },
    /// the view goes to moment `t` of the recording (the present when `t` is now or later);
    /// the devices keep their present state
    Seek(Ns),
}

/// what a tick or command did
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    pub events: usize,
    pub now: Ns,
    /// call tick again after this many milliseconds (None: nothing to do until a command)
    pub wake_ms: Option<u32>,
    /// frames were sent since the last tick
    pub new_hops: bool,
    /// the moment the view shows while it stands in the past (None: the present)
    pub view: Option<Ns>,
}

pub struct Inline {
    pub eng: Engine,
    mode: Mode,
    anchor: Option<(u64, Ns)>,
    /// the moment the view shows (None: the present)
    view: Option<Ns>,
    /// events per tick at most
    pub budget: usize,
}

impl Inline {
    pub fn new(eng: Engine) -> Inline {
        Inline { eng, mode: Mode::Paused, anchor: None, view: None, budget: 50_000 }
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    /// the moment the view shows while it stands in the past (None: the present)
    pub fn view(&self) -> Option<Ns> {
        self.view
    }
    /// the moment the view shows: a moment of the recording or the engine's clock
    pub fn view_time(&self) -> Ns {
        self.view.unwrap_or(self.eng.now())
    }

    fn done(&self, events: usize, hops_before: u64) -> Tick {
        let wake_ms = match self.mode {
            Mode::Realtime { .. } => Some(16),
            _ => None,
        };
        Tick { events, now: self.eng.now(), wake_ms, new_hops: self.eng.trace.next_hop_id() != hops_before, view: self.view }
    }

    /// a command from the user interface
    pub fn send(&mut self, c: Cmd, wall_us: u64) -> Result<Tick, ActErr> {
        let hops = self.eng.trace.next_hop_id();
        let mut n = 0;
        if matches!(c, Cmd::Realtime { .. } | Cmd::StepEvent | Cmd::RunFor(_) | Cmd::RunIdle) {
            // the engine moves on: the view follows it
            self.view = None;
        }
        match c {
            Cmd::Realtime { speed_pm } => {
                self.mode = Mode::Realtime { speed_pm: speed_pm.max(1) };
                self.anchor = Some((wall_us, self.eng.now()));
            }
            Cmd::Pause => {
                self.mode = Mode::Paused;
                self.anchor = None;
            }
            Cmd::Simulation => {
                self.mode = Mode::Simulation;
                self.anchor = None;
            }
            Cmd::StepEvent => {
                if self.eng.step().is_some() {
                    n = 1;
                }
            }
            Cmd::StepHop { mask } => n = self.step(&Filter::protocols(mask)),
            Cmd::Step(f) => n = self.step(&f),
            Cmd::StepBack(f) => {
                let at = self.view_time();
                if let Some(t) = self.eng.trace.prev_send(at, &f) {
                    self.view = Some(t);
                }
            }
            Cmd::Live => self.view = None,
            Cmd::RunFor(d) => {
                let t = self.eng.now() + d;
                while self.eng.run_until(t, self.budget) == RunOut::Budget {
                    n += self.budget;
                }
            }
            Cmd::RunIdle => {
                while self.eng.run_idle(self.budget) == RunOut::Budget {
                    n += self.budget;
                }
            }
            Cmd::Apply(a) => self.eng.apply(a)?,
            Cmd::Cli { dev, line } => self.eng.cli_line(dev, &line)?,
            Cmd::Interrupt { dev } => self.eng.cli_interrupt(dev)?,
            Cmd::Seek(t) => self.view = if t < self.eng.now() { Some(t) } else { None },
        }
        if let Mode::Realtime { .. } = self.mode
            && let Some(a) = self.anchor.as_mut()
            && n > 0
        {
            // steps in realtime mode move the anchor with the clock
            *a = (wall_us, self.eng.now());
        }
        Ok(self.done(n, hops))
    }

    /// a step forward: through the recording while the view stands in the past, else the
    /// engine runs to the next frame the filter passes
    fn step(&mut self, f: &Filter) -> usize {
        if let Some(v) = self.view {
            let now = self.eng.now();
            match self.eng.trace.next_send(v, now, f) {
                Some(t) => {
                    self.view = if t < now { Some(t) } else { None };
                    return 0;
                }
                None => self.view = None,
            }
        }
        self.step_hop(f)
    }

    /// runs until the next frame the filter passes was sent
    fn step_hop(&mut self, f: &Filter) -> usize {
        let mut n = 0;
        while n < self.budget {
            let before = self.eng.trace.next_hop_id();
            if self.eng.step().is_none() {
                break;
            }
            n += 1;
            let hit = self.eng.trace.hops_after(before.saturating_sub(1)).any(|h| h.id >= before && f.hop(h));
            if hit {
                break;
            }
        }
        n
    }

    /// advances realtime mode to the wall clock
    pub fn tick(&mut self, wall_us: u64) -> Tick {
        let hops = self.eng.trace.next_hop_id();
        let Mode::Realtime { speed_pm } = self.mode else {
            return self.done(0, hops);
        };
        let (aw, an) = *self.anchor.get_or_insert((wall_us, self.eng.now()));
        let target = an + wall_us.saturating_sub(aw).saturating_mul(speed_pm as u64);
        let before = self.eng.events_done();
        if self.eng.run_until(target, self.budget) == RunOut::Budget {
            self.anchor = Some((wall_us, self.eng.now()));
        }
        self.done((self.eng.events_done() - before) as usize, hops)
    }
}

/// what the engine has to say besides the clock: console output and what the consoles
/// changed (Inline::reports, Engine::take_reports)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    /// output of a device's console at virtual time `at` (UTF-8, "\n" line ends); the prompt
    /// and the echo of typed lines are the terminal's (Engine::cli_prompt)
    CliOut { dev: u32, at: Ns, bytes: alloc::vec::Vec<u8> },
    /// the console's prompt changed, or a command started (busy: typed lines wait) or ended
    CliState { dev: u32, prompt: alloc::string::String, busy: bool },
    /// "copy running-config startup-config": the configuration the device boots with now
    Startup { dev: u32, text: alloc::string::String },
    /// "hostname" typed at the console: the device's new name (not while a startup
    /// configuration boots it)
    Hostname { dev: u32, name: alloc::string::String },
    /// a line of a device's log at virtual time `at`: what its services log (syslog), the
    /// IOS console messages (%LINK, %LINEPROTO, %SYS) and debugging output; `sev` 0
    /// (emergency) .. 7 (debugging), `tag` the program or the message code
    Log { dev: u32, at: Ns, sev: u8, tag: alloc::string::String, text: alloc::string::String },
    /// a device's files changed (a transfer, a command line): the path and every record now,
    /// for the project (files.rs)
    Files { dev: u32, at: Ns, path: alloc::string::String, files: alloc::vec::Vec<jfn_model::files::File> },
}

impl Inline {
    /// what happened since the last call, in order
    pub fn reports(&mut self) -> alloc::vec::Vec<Report> {
        self.eng.take_reports()
    }
}
