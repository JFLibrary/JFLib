// wnd_dock -- PaintCraft: places the four tool windows (Werkzeuge, Verlauf, Ebenen, Farben)
// inside the main window, every frame, so they follow moves and resizes of the main window.
//                                              [apps/PaintCraft/src/wnd_dock.s]
//
//   dock_frame()          (first of the module frames) the layout of this frame
//   dock_hidden(WND_*) -> true when the layout hides that window this frame (no room)
//   dock_horiz() -> true when the Werkzeuge window is laid out in two rows (low window)
//   dock_reset(WND_*)     the window goes back to the automatic layout
//   dock_rec(WND_*) -> &mut DockRec (settings.rs persists mode and anchors)
//
// Modes of a window:
//   DM_AUTO    the default layout: Werkzeuge top left, Farben bottom left (right of the
//              Werkzeuge when the main window is low), Verlauf top right, Ebenen bottom right;
//              the Verlauf hides when it does not fit above the Ebenen, a very small main window
//              shows only Werkzeuge (left) and Farben (right); a low one lays the Werkzeuge out
//              in two rows.
//   DM_ANCHOR  the user dragged it inside the main window: it keeps its distance from the
//              nearest horizontal and vertical edge (so it stays at "its" edge on resizes).
//   DM_FREE    the user dragged it out of the main window (centre outside): it stays where it
//              is on the screen (the library); on Windows kept inside the monitor's work area.
// Docked windows (auto / anchor) never leave the main window's client area.
// A user move is a WRF_CHANGED rect whose position differs from the one applied last by more
// than DK_TOL DIP (the platform echoes our own moves, rounded).

use ui::*;

use crate::app::constants::*;
use crate::app::Pn;

const DK_M: i32 = 8; // margin from the edges (DIP)
const DK_TOP_DEF: i32 = 132; // canvas top / status bar when the canvas is unknown
const DK_SB_DEF: i32 = 32;
const DK_TOL: i32 = 2;
const DK_TINY_W: i32 = 640; // canvas area below this: only Werkzeuge + Farben
const DK_TINY_H: i32 = 360;
const WT_V_W: i32 = 74; // Werkzeuge client size (wnd_tools.rs): 2 x 10 buttons
const WT_V_H: i32 = 346;
const WT_H_W: i32 = 346; //   10 x 2
const WT_H_H: i32 = 74;

/// how the layout places a tool window (DOCKREC)
#[derive(Clone, Copy, Default)]
pub(crate) struct DockRec {
    /// DR_MODE: DM_*
    pub(crate) mode: i32,
    /// DR_AX: 0 left edge, 1 right edge; DR_DX: distance from it (DIP)
    pub(crate) ax: i32,
    pub(crate) dx: i32,
    /// DR_AY: 0 top, 1 bottom; DR_DY
    pub(crate) ay: i32,
    pub(crate) dy: i32,
    /// DR_APX / DR_APY: position applied last (DIP)
    pub(crate) apx: i32,
    pub(crate) apy: i32,
    /// DR_HIDE: hidden by the layout this frame
    pub(crate) hide: bool,
}

/// wnd_dock.s
#[derive(Default)]
pub(crate) struct Dock {
    /// g_dk: one record per tool window (WND_*)
    pub(crate) dk: [DockRec; WND_COUNT as usize],
    /// g_dk_horiz: the Werkzeuge in two rows
    pub(crate) horiz: bool,
}

/// dock_rec(k) -> &DOCKREC of window k (k out of range -> window 0)
pub(crate) fn dock_rec(d: &mut Dock, k: i32) -> &mut DockRec {
    let k = if (k as u32) < WND_COUNT as u32 { k as usize } else { 0 };
    &mut d.dk[k]
}

/// dock_hidden(k) -> true when hidden by the layout
pub(crate) fn dock_hidden(d: &Dock, k: i32) -> bool {
    let k = if (k as u32) < WND_COUNT as u32 { k as usize } else { 0 };
    d.dk[k].hide
}

/// dock_horiz() -> true = Werkzeuge in two rows
pub(crate) fn dock_horiz(d: &Dock) -> bool {
    d.horiz
}

/// dock_reset(k) -- automatic layout again
pub(crate) fn dock_reset(d: &mut Dock, k: i32) {
    let r = dock_rec(d, k);
    r.mode = DM_AUTO;
    r.apx = i32::MIN;
}

// dk_wrect(k) -> the WINRECT of window k (the modules' getters)
fn dk_wrect(p: &Pn, k: i32) -> Shared<WinRect> {
    match k {
        WND_HISTORY => crate::workspace::panels::history::whist_rect(&p.whist),
        WND_LAYERS => crate::workspace::panels::layers::wlay_rect(&p.wlay),
        WND_COLORS => crate::workspace::panels::colors::wcol_rect(&p.wcol),
        _ => crate::workspace::panels::tools::wtools_rect(&p.wtools),
    }
}

// the frame's layout values (dock_frame's locals; dk_classify reads cw / ch / ow / oh)
struct Frame {
    cw: i32,
    ch: i32,
    ow: [i32; 4],
    oh: [i32; 4],
}

/// dock_frame() -- sizes, user moves, the automatic layout, then every docked window's place
pub(crate) fn dock_frame(p: &mut Pn, ui: &mut Ui) {
    let (cw, ch) = ui.client_size();
    let mut f = Frame { cw, ch, ow: [0; 4], oh: [0; 4] };
    let mut ax = [0i32; 4];
    let mut ay = [0i32; 4];
    // the canvas area (below the tool bars, above the status bar)
    let mut t = DK_TOP_DEF + DK_M;
    let mut b = ch.wrapping_sub(DK_SB_DEF + DK_M);
    let cr = crate::workspace::view::view_canvas_rect(&p.view);
    if cr[2] > 0 && cr[3] > 0 {
        t = ui.px2dip(cr[1]) + DK_M;
        b = ui.px2dip(cr[1] + cr[3]) - DK_M;
    }
    let l = DK_M;
    let r = cw - DK_M;
    let tbh = ui.style.metric(SM_WIN_TBH_TOOL) + 2; // title bar + border
    // Werkzeuge in two rows when the vertical window does not fit the height
    let hgt = b.wrapping_sub(t);
    p.dock.horiz = hgt < WT_V_H + tbh;
    // very small: only Werkzeuge and Farben
    let tiny = hgt < DK_TINY_H || r - l < DK_TINY_W;
    // outer sizes
    for k in 0..WND_COUNT {
        let wr = dk_wrect(p, k).get();
        let mut w = wr.w;
        let mut h = wr.h;
        if let Some(cur) = ui.window_rect(k as u32 + WINID_TOOLS) {
            if cur.h > 0 {
                h = cur.h;
            }
        }
        if k == WND_TOOLS {
            w = WT_V_W;
            h = WT_V_H;
            if p.dock.horiz {
                w = WT_H_W;
                h = WT_H_H;
            }
        }
        if h <= 0 {
            h = 320; // (Farben before its first layout)
        }
        f.ow[k as usize] = w + 2;
        f.oh[k as usize] = h + tbh;
    }
    // user moves: a WRF_CHANGED position away from the applied one
    for k in 0..WND_COUNT {
        let sr = dk_wrect(p, k);
        let mut wr = sr.get();
        if wr.flags & WRF_CHANGED == 0 {
            continue;
        }
        wr.flags &= !WRF_CHANGED;
        sr.set(wr);
        if wr.flags & WRF_POS == 0 {
            continue;
        }
        let d = p.dock.dk[k as usize];
        let user = match wr.x.checked_sub(d.apx) {
            None => true,
            Some(v) if v.wrapping_abs() > DK_TOL => true,
            _ => match wr.y.checked_sub(d.apy) {
                None => true,
                Some(v) => v.wrapping_abs() > DK_TOL,
            },
        };
        if user {
            dk_classify(k, &wr, &mut p.dock.dk[k as usize], &f);
        }
    }
    // the automatic layout
    ax[0] = l; // Werkzeuge: top left
    ay[0] = t;
    ax[1] = r - f.ow[1]; // Verlauf: top right
    ay[1] = t;
    ax[2] = r - f.ow[2]; // Ebenen: bottom right
    ay[2] = b - f.oh[2];
    // Farben: bottom left, right of the Werkzeuge when they would overlap; bottom right in a
    // very small window
    let cy = b - f.oh[3];
    ay[3] = cy;
    let mut cx = l;
    if tiny {
        cx = r - f.ow[3];
    } else if cy < t + f.oh[0] + DK_M {
        // only when the Werkzeuge are there (auto)
        let open = crate::workspace::chrome::chrome_wnd_open(&p.chrome, WND_TOOLS).get();
        if open != 0 && p.dock.dk[WND_TOOLS as usize].mode == DM_AUTO {
            cx = l + f.ow[0] + DK_M;
        }
    }
    ax[3] = cx;
    // hiding: very small -> no Verlauf / Ebenen; Verlauf above the Ebenen only with room
    for d in p.dock.dk.iter_mut() {
        d.hide = false;
    }
    let dk = &mut p.dock.dk;
    if tiny {
        if dk[WND_HISTORY as usize].mode != DM_FREE {
            dk[WND_HISTORY as usize].hide = true;
        }
        if dk[WND_LAYERS as usize].mode != DM_FREE {
            dk[WND_LAYERS as usize].hide = true;
        }
    } else if dk[WND_HISTORY as usize].mode == DM_AUTO
        && dk[WND_LAYERS as usize].mode == DM_AUTO
        && crate::workspace::chrome::chrome_wnd_open(&p.chrome, WND_LAYERS).get() != 0
        && t + f.oh[1] + DK_M > ay[2]
    {
        p.dock.dk[WND_HISTORY as usize].hide = true;
    }
    // place
    for k in 0..WND_COUNT as usize {
        let sr = dk_wrect(p, k as i32);
        let d = p.dock.dk[k];
        if d.mode == DM_FREE {
            dk_onscreen(ui, &sr, &mut p.dock.dk[k], k as i32);
            continue;
        }
        if d.hide {
            continue;
        }
        let (mut x, mut y);
        if d.mode == DM_ANCHOR {
            x = d.dx;
            if d.ax != 0 {
                x = cw - f.ow[k] - d.dx;
            }
            y = d.dy;
            if d.ay != 0 {
                y = ch - f.oh[k] - d.dy;
            }
        } else {
            x = ax[k];
            y = ay[k];
        }
        // inside the client area: x in 0 .. cw - w, y in 0 .. ch - h (0 when it is larger)
        x = x.min(cw - f.ow[k]).max(0);
        y = y.min(ch - f.oh[k]).max(0);
        // apply when it changed (or was never placed)
        let mut wr = sr.get();
        if wr.flags & WRF_POS != 0 && x == d.apx && y == d.apy {
            continue;
        }
        wr.x = x;
        wr.y = y;
        wr.flags |= WRF_POS;
        sr.set(wr);
        let dd = &mut p.dock.dk[k];
        dd.apx = x;
        dd.apy = y;
    }
}

// dk_classify(k, &WINRECT, &DOCKREC, &frame) -- the user moved window k: free when its centre
// left the client area, else anchored to the nearest edges
fn dk_classify(k: i32, wr: &WinRect, d: &mut DockRec, f: &Frame) {
    d.apx = wr.x;
    d.apy = wr.y;
    let ow = f.ow[k as usize];
    let oh = f.oh[k as usize];
    let cx = (ow >> 1).wrapping_add(wr.x);
    let cy = (oh >> 1).wrapping_add(wr.y);
    if cx < 0 || cx > f.cw || cy < 0 || cy > f.ch {
        d.mode = DM_FREE;
        return;
    }
    d.mode = DM_ANCHOR;
    // horizontal edge
    d.ax = (cx.wrapping_add(cx) > f.cw) as i32;
    d.dx = if d.ax != 0 { f.cw - ow - wr.x } else { wr.x };
    // vertical edge
    d.ay = (cy.wrapping_add(cy) > f.ch) as i32;
    d.dy = if d.ay != 0 { f.ch - oh - wr.y } else { wr.y };
}

// dk_onscreen(&WINRECT, &DOCKREC, k) -- (Windows) a free window stays inside the work area of
// its monitor (e.g. the Farben window growing with Mehr >> at the right screen edge)
fn dk_onscreen(ui: &mut Ui, sr: &Shared<WinRect>, d: &mut DockRec, k: i32) {
    let wr = sr.get();
    d.apx = wr.x; // (our own moves are not the user's)
    d.apy = wr.y;
    if !cfg!(windows) {
        return;
    }
    let id = k as u32 + WINID_TOOLS;
    if ui.window_state(id) & UWS_VISIBLE == 0 {
        return;
    }
    let (i, pw) = ui.window_pw(id);
    if i <= 0 || pw < 0 {
        return;
    }
    let Some(r) = plat::win_get_rect(pw) else { return };
    let Some(a) = plat::win_work_area(pw) else { return };
    if a[2] <= 0 {
        return;
    }
    // horizontal shift (px): right edge first, then the left edge wins
    let mut sx = 0i32;
    let v = (a[0] + a[2]) - (r[0] + r[2]);
    if v < 0 {
        sx = v;
    }
    let v = a[0] - (r[0] + sx);
    if v > 0 {
        sx += v;
    }
    let mut sy = 0i32;
    let v = (a[1] + a[3]) - (r[1] + r[3]);
    if v < 0 {
        sy = v;
    }
    let v = a[1] - (r[1] + sy);
    if v > 0 {
        sy += v;
    }
    if sx | sy == 0 {
        return;
    }
    let mut wr = sr.get();
    wr.x += ui.px2dip(sx);
    wr.y += ui.px2dip(sy);
    sr.set(wr);
    d.apx = wr.x;
    d.apy = wr.y;
}
