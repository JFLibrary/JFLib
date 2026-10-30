// linux_wl_act -- xdg-activation-v1: windows brought to the front
//
// A Wayland client can neither raise nor un-minimize its toplevels; it asks the compositor with
// a token that was handed out for an action of the user (xdg_activation_v1.activate(token,
// surface)).  Without one KWin and mutter only let the window demand attention, and KWin 6
// gives a new window the keyboard only with one.  The tokens come from
//   start      $XDG_ACTIVATION_TOKEN, the launcher's (Plasma's menu, KRunner, Dolphin, GNOME's
//              shell): read and removed from the environment at the connection (programs the
//              app starts do not get it), used once for the first toplevel that is mapped
//   provided   act_provide(token), from any thread: KDE's tray (StatusNotifierItem
//              .ProvideXdgActivationToken before Activate) and notifications (the signal
//              ActivationToken before ActionInvoked); the next win_activate within
//              PROVIDED_US takes it, an older one is dropped
//   own        win_activate without a provided one: get_activation_token with the serial of
//              the last input event, the surface that has the keyboard and the app id; the
//              compositor grants it while the app is active (a dialog or another window of
//              the app raised by a click), otherwise activate only makes it demand attention
// win_activate(w) of a toplevel that is active already does nothing; of one that is not
// mapped yet (shown again a moment ago) waits for its first buffer.  One token request is in
// flight at a time; its answer goes to the window asked for last.

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

use super::proto::{Ev, Iface, Req};
use super::wire::{Args, A};
use super::Wl;
use crate::{PW_MAX, WST_ACTIVE, WST_MINIMIZED};

/// how long a provided token waits for win_activate (the tray and the notification server
/// send it right before the click that leads to it)
const PROVIDED_US: u64 = 10_000_000;

pub(crate) static IFACE_XDG_ACTIVATION: Iface = Iface { name: "xdg_activation_v1", evs: &[] };

static IFACE_ACT_TOKEN: Iface = Iface { name: "xdg_activation_token_v1", evs: &[Ev { sig: "done:s", h: Some(wlh_act_done) }] };

const ACT_GET_TOKEN: Req = Req { op: 1, sig: "get_activation_token:n", destroy: false };
const ACT_ACTIVATE: Req = Req { op: 2, sig: "activate:so", destroy: false };
const TOKEN_SET_SERIAL: Req = Req { op: 0, sig: "set_serial:uo", destroy: false };
const TOKEN_SET_APP_ID: Req = Req { op: 1, sig: "set_app_id:s", destroy: false };
const TOKEN_SET_SURFACE: Req = Req { op: 2, sig: "set_surface:o", destroy: false };
const TOKEN_COMMIT: Req = Req { op: 3, sig: "commit:", destroy: false };
const TOKEN_DESTROY: Req = Req { op: 4, sig: "destroy:", destroy: true };

/// a token another thread was given for us, with the time it came
static PROVIDED: SpinLock<Option<(Vec<u8>, u64)>> = SpinLock::new(None);

/// (tray, notifications) a token for the next win_activate
pub(crate) fn act_provide(token: &str) {
    if !token.is_empty() {
        *PROVIDED.lock() = Some((token.as_bytes().to_vec(), crate::time_us()));
    }
}

fn provided_take() -> Option<Vec<u8>> {
    let (t, at) = PROVIDED.lock().take()?;
    if crate::time_us().saturating_sub(at) <= PROVIDED_US { Some(t) } else { None }
}

pub(crate) struct Act {
    start: Option<Vec<u8>>, // the launcher's token until the first toplevel is mapped
    token: Option<Vec<u8>>, // a token at hand for `want`
    want: i32,              // the window to activate (-1 = none)
    req: u32,               // xdg_activation_token_v1 in flight (0 = none)
}

impl Default for Act {
    fn default() -> Self {
        Act { start: None, token: None, want: -1, req: 0 }
    }
}

impl Wl {
    /// (connect) the launcher's token, out of the environment
    pub(crate) fn act_start(&mut self) {
        if let Some(t) = crate::env("XDG_ACTIVATION_TOKEN") {
            crate::setenv("XDG_ACTIVATION_TOKEN", None);
            if !t.is_empty() {
                self.act.start = Some(t.into_bytes());
            }
        }
    }

    /// (present) window i got its first buffer since it was shown: the launcher's token for
    /// the first toplevel, or the activation it waits for
    pub(crate) fn act_mapped(&mut self, i: usize) {
        if self.win[i].top == 0 {
            return; // child windows and popups have no activation of their own
        }
        if let Some(t) = self.act.start.take() {
            if self.g.actm != 0 {
                self.w.send(self.g.actm, &ACT_ACTIVATE, &[A::S(Some(&t)), A::O(self.win[i].surf)]);
            }
        }
        self.act_try();
    }

    /// win_activate(w) of a toplevel: activate it (a token provided, or one asked for)
    pub(crate) fn act_request(&mut self, w: usize) {
        if self.g.actm == 0 {
            return;
        }
        let st = self.win[w].state;
        if st & WST_ACTIVE != 0 && st & WST_MINIMIZED == 0 && self.win[w].surf != 0 {
            return;
        }
        self.act.want = w as i32;
        if let Some(t) = provided_take() {
            self.act.token = Some(t);
            self.act_try();
            return;
        }
        if self.act.req != 0 {
            return; // the answer to the one in flight goes to `want`
        }
        let tok = self.w.send(self.g.actm, &ACT_GET_TOKEN, &[A::N(&IFACE_ACT_TOKEN)]);
        self.act.req = tok;
        if self.inp.input_serial != 0 && self.g.seat != 0 {
            self.w.send(tok, &TOKEN_SET_SERIAL, &[A::U(self.inp.input_serial), A::O(self.g.seat)]);
        }
        // the surface that has the keyboard: the compositor grants tokens to the active window
        let kw = self.inp.kb_win;
        if self.inp.kb_focus && (0..PW_MAX as i32).contains(&kw) && self.win[kw as usize].surf != 0 {
            self.w.send(tok, &TOKEN_SET_SURFACE, &[A::O(self.win[kw as usize].surf)]);
        }
        let id = app_id();
        if !id.is_empty() {
            self.w.send(tok, &TOKEN_SET_APP_ID, &[A::S(Some(id.as_bytes()))]);
        }
        self.w.send(tok, &TOKEN_COMMIT, &[]);
        self.w.flush();
    }

    /// a token at hand and the window it is for mapped: activate it
    fn act_try(&mut self) {
        let w = self.act.want;
        if w < 0 || self.rec(w).is_none() || self.act.token.is_none() {
            return;
        }
        let r = self.win[w as usize];
        if r.surf == 0 || r.top == 0 || !r.mapped {
            return; // when it is mapped (act_mapped)
        }
        let Some(t) = self.act.token.take() else { return };
        self.act.want = -1;
        self.w.send(self.g.actm, &ACT_ACTIVATE, &[A::S(Some(&t)), A::O(r.surf)]);
        self.w.flush();
    }
}

/// xdg_activation_token_v1.done(token): the compositor's token for our request
fn wlh_act_done(wl: &mut Wl, id: u32, a: &Args, _u: u64) {
    wl.w.send(id, &TOKEN_DESTROY, &[]);
    if wl.act.req == id {
        wl.act.req = 0;
    }
    if let Some(t) = a.s(0) {
        wl.act.token = Some(t.to_vec());
    }
    wl.act_try();
}

/// the app id our toplevels carry: the basename of argv[0] (objects_create)
pub(crate) fn app_id() -> String {
    let args = crate::args();
    let Some(a0) = args.first() else { return String::new() };
    String::from(match a0.rfind('/') {
        Some(n) => &a0[n + 1..],
        None => &a0[..],
    })
}
