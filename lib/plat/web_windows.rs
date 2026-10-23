// Library windows and popups composited into one browser canvas. Every surface is drawn
// by the existing software renderer; the browser host receives pixels, not another UI.
// The windows keep their pixels as R, G, B, A rows (the order of the browser's ImageData and
// Android's Bitmap): the frames' B, G, R, x is swapped once, while copying the dirty
// rectangles in, and the host copies rows without touching pixels.  One window over the
// whole canvas without popups is the canvas itself (no composition); otherwise the windows
// and popups are composited into the canvas buffer.

use crate::*;
use alloc::{string::String, vec::Vec};
use rt::sync::SpinLock;

struct Window {
    rect: [i32; 4],
    min: (i32, i32),
    dpi: i32,
    title: String,
    shown: bool,
    enabled: bool,
    state: u32,
    pixels: Vec<u8>, // R, G, B, A rows of width * 4 bytes
    width: i32,
    height: i32,
}
struct Popup {
    owner: i32,
    rect: [i32; 4],
    pixels: Vec<u8>,
    width: i32,
    height: i32,
}
struct Display {
    width: i32,
    height: i32,
    dpi: i32,
    active: i32,
    capture: Option<i32>,
    windows: [Option<Window>; PW_MAX],
    popups: [Option<Popup>; PPW_MAX as usize],
    dirty: bool,
    damage: Vec<[i32; 4]>,
    composite: Vec<u8>, // the canvas (R, G, B, A rows) when more than one surface shows
    composed: bool,     // it holds the last composition (the single-window path skips it)
    cursor: u32,
    pointer: (i32, i32),
    drag: Option<(i32, i32, i32, [i32; 4], u32)>,
}
static DISPLAY: SpinLock<Display> = SpinLock::new(Display {
    width: 1000,
    height: 700,
    dpi: 96,
    active: 0,
    capture: None,
    windows: [const { None }; PW_MAX],
    popups: [const { None }; PPW_MAX as usize],
    dirty: false,
    damage: Vec::new(),
    composite: Vec::new(),
    composed: false,
    cursor: CUR_ARROW,
    pointer: (0, 0),
    drag: None,
});

fn window_mut(display: &mut Display, id: i32) -> Option<&mut Window> {
    display.windows.get_mut(usize::try_from(id).ok()?)?.as_mut()
}
fn window(display: &Display, id: i32) -> Option<&Window> {
    display.windows.get(usize::try_from(id).ok()?)?.as_ref()
}
pub(super) fn viewport(width: i32, height: i32, dpi: i32) {
    let mut display = DISPLAY.lock();
    display.width = width.clamp(1, 16384);
    display.height = height.clamp(1, 16384);
    display.dpi = dpi.clamp(48, 480);
}
/// The window typed text goes to (as keys do): the active one while it is shown and enabled.
pub(super) fn text_target() -> Option<u8> {
    let d = DISPLAY.lock();
    window(&d, d.active).is_some_and(|w| w.shown && w.enabled).then_some(d.active as u8)
}

/// Create a window within the app canvas; window zero uses the host viewport.
pub fn win_create(
    title: &str,
    width: i32,
    height: i32,
    _flags: u32,
    _owner: i32,
    x: i32,
    y: i32,
) -> i32 {
    let mut display = DISPLAY.lock();
    let Some(id) = display.windows.iter().position(Option::is_none) else {
        return -1;
    };
    let dpi = display.dpi;
    let width = if id == 0 {
        display.width
    } else {
        (width.max(1) as i64 * dpi as i64 / 96).clamp(1, 16384) as i32
    };
    let height = if id == 0 {
        display.height
    } else {
        (height.max(1) as i64 * dpi as i64 / 96).clamp(1, 16384) as i32
    };
    let x = if id == 0 {
        0
    } else if x == PW_DEFAULT {
        ((display.width - width) / 2).max(0)
    } else {
        x
    };
    let y = if id == 0 {
        0
    } else if y == PW_DEFAULT {
        ((display.height - height) / 2).max(0)
    } else {
        y
    };
    display.windows[id] = Some(Window {
        rect: [x, y, width, height],
        min: (1, 1),
        dpi,
        title: String::from(title),
        shown: false,
        enabled: true,
        // Android's main surface fills its Activity; desktop caption controls and resize
        // grips belong only to floating library windows, not to the phone viewport.
        state: if id == 0 && cfg!(target_os = "android") { WST_MAXIMIZED } else { WST_CSD },
        pixels: Vec::new(),
        width: 0,
        height: 0,
    });
    display.dirty = true;
    drop(display);
    if id == 0 { super::notice(super::REQUEST_TITLE, title.as_bytes()); }
    id as i32
}
/// Destroy a library surface and return focus to the main window.
pub fn win_destroy(id: i32) {
    let mut display = DISPLAY.lock();
    if let Some(slot) = usize::try_from(id)
        .ok()
        .and_then(|id| display.windows.get_mut(id))
    {
        *slot = None;
    }
    if display.active == id {
        display.active = 0;
    }
    display.dirty = true;
}
/// Show or hide a library surface.
pub fn win_show(id: i32, mode: u32) {
    let mut display = DISPLAY.lock();
    if let Some(window) = window_mut(&mut display, id) {
        let shown = mode != PWS_HIDE;
        if window.shown != shown {
            window.shown = shown;
            display.dirty = true;
        }
    }
    if mode == PWS_SHOW {
        display.active = id;
    }
}
/// Set keyboard focus to an enabled library window.
pub fn win_activate(id: i32) {
    let mut display = DISPLAY.lock();
    if window(&display, id).is_some_and(|w| w.enabled) {
        display.active = id;
    }
}
/// Enable or disable window input, including modality established by the UI runtime.
pub fn win_enable(id: i32, enabled: bool) {
    if let Some(window) = window_mut(&mut DISPLAY.lock(), id) {
        window.enabled = enabled;
    }
}
/// Change a library surface rectangle, preserving fields equal to PW_KEEP.
pub fn win_set_rect(id: i32, x: i32, y: i32, width: i32, height: i32) {
    let mut display = DISPLAY.lock();
    if let Some(window) = window_mut(&mut display, id) {
        let before = window.rect;
        for (i, value) in [x, y, width, height].iter().enumerate() {
            if *value != PW_KEEP {
                window.rect[i] = if i < 2 {
                    *value
                } else {
                    (*value).clamp(1, 16384)
                };
            }
        }
        if before != window.rect { display.dirty = true; }
    }
}
/// Current canvas-relative client rectangle.
pub fn win_get_rect(id: i32) -> Option<[i32; 4]> {
    window(&DISPLAY.lock(), id).map(|w| w.rect)
}
/// The browser canvas is the available work area.
pub fn win_work_area(_id: i32) -> Option<[i32; 4]> {
    let d = DISPLAY.lock();
    Some([0, 0, d.width, d.height])
}
/// Update a library title and the browser document title for window zero.
pub fn win_set_title(id: i32, title: &str) {
    if let Some(window) = window_mut(&mut DISPLAY.lock(), id) {
        if window.title == title { return; }
        window.title = String::from(title);
    }
    if id == 0 {
        super::notice(super::REQUEST_TITLE, title.as_bytes());
    }
}
/// Current library state flags; browser surfaces use library-owned chrome.
pub fn win_state(id: i32) -> u32 {
    let display = DISPLAY.lock();
    window(&display, id).map_or(0, |w| {
        w.state | if display.active == id { WST_ACTIVE } else { 0 }
    })
}
/// Apply library-window commands; close is delivered to the normal app close workflow.
pub fn win_cmd(id: i32, command: u32) {
    if command == WCMD_CLOSE {
        let _ = evq_push(Event {
            typ: E_CLOSE,
            win: id as u8,
            ..Event::default()
        });
        return;
    }
    let mut display = DISPLAY.lock();
    let size = (display.width, display.height);
    if let Some(window) = window_mut(&mut display, id) {
        match command {
            WCMD_MAXIMIZE => {
                window.rect = [0, 0, size.0, size.1];
                window.state |= WST_MAXIMIZED;
            }
            WCMD_RESTORE => {
                window.state &= !(WST_MAXIMIZED | WST_MINIMIZED);
                window.shown = true;
            }
            WCMD_MINIMIZE if id != 0 => {
                window.state |= WST_MINIMIZED;
                window.shown = false;
            }
            _ => {}
        }
        display.dirty = true;
    }
}
/// Begin a pointer-driven move of a non-main library window.
pub fn win_begin_move(id: i32) {
    let mut d = DISPLAY.lock();
    if id != 0 {
        if let Some(rect) = window(&d, id).map(|w| w.rect) {
            d.drag = Some((id, d.pointer.0, d.pointer.1, rect, 0));
        }
    }
}
/// Begin resizing a library surface inside the browser canvas.
pub fn win_begin_resize(id: i32, edges: u32) {
    let mut d = DISPLAY.lock();
    if id != 0 {
        if let Some(rect) = window(&d, id).map(|w| w.rect) {
            d.drag = Some((id, d.pointer.0, d.pointer.1, rect, edges));
        }
    }
}
/// Physical client dimensions.
pub fn win_size(id: i32) -> (i32, i32) {
    let d = DISPLAY.lock();
    window(&d, id).map_or((d.width, d.height), |w| (w.rect[2], w.rect[3]))
}
/// Current host DPI of a library surface.
pub fn win_dpi(id: i32) -> i32 {
    let d = DISPLAY.lock();
    window(&d, id).map_or(d.dpi, |w| w.dpi)
}
/// Minimum drag-resize dimensions in DIP.
pub fn win_set_min_size(id: i32, width: i32, height: i32) {
    if let Some(window) = window_mut(&mut DISPLAY.lock(), id) {
        window.min = (width.max(1), height.max(1));
    }
}

fn frame_size(frame: &Frame<'_>) -> Option<(usize, usize)> {
    if frame.w <= 0
        || frame.h <= 0
        || frame.w > 16384
        || frame.h > 16384
        || frame.stride < frame.w.checked_mul(4)?
    {
        return None;
    }
    let row = frame.w as usize * 4;
    let len = row.checked_mul(frame.h as usize)?;
    let last = (frame.h as usize - 1).checked_mul(frame.stride as usize)?.checked_add(row)?;
    if len > 256 << 20 || last > frame.px.len() {
        return None;
    }
    Some((row, len))
}
/// B, G, R, x pixels -> R, G, B, 255 (equal lengths): whole words (a byte swap and a rotation,
/// which the compiler turns into vector shuffles) where both are 4-byte aligned
fn to_rgba(dst: &mut [u8], src: &[u8]) {
    // SAFETY: u32 is plain data; align_to only reinterprets the aligned middle parts.
    let ((dp, dm, _), (sp, sm, _)) = unsafe { (dst.align_to_mut::<u32>(), src.align_to::<u32>()) };
    if dp.is_empty() && sp.is_empty() && dm.len() == sm.len() && dm.len() * 4 == src.len() {
        for (d, s) in dm.iter_mut().zip(sm) {
            *d = (u32::from_le(*s).swap_bytes().rotate_right(8) | 0xFF00_0000).to_le();
        }
        return;
    }
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        d.copy_from_slice(&[s[2], s[1], s[0], 255]);
    }
}
fn pixels(frame: &Frame<'_>) -> Option<Vec<u8>> {
    let (row, len) = frame_size(frame)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).ok()?;
    bytes.resize(len, 0);
    for y in 0..frame.h as usize {
        let start = y.checked_mul(frame.stride as usize)?;
        to_rgba(&mut bytes[y * row..y * row + row], frame.px.get(start..start + row)?);
    }
    Some(bytes)
}
fn clipped(rect: [i32; 4], width: i32, height: i32) -> Option<[i32; 4]> {
    let x = rect[0].clamp(0, width);
    let y = rect[1].clamp(0, height);
    let right = rect[0].saturating_add(rect[2].max(0)).clamp(0, width);
    let bottom = rect[1].saturating_add(rect[3].max(0)).clamp(0, height);
    (right > x && bottom > y).then_some([x, y, right - x, bottom - y])
}
fn damage(display: &mut Display, rect: [i32; 4]) {
    if display.dirty { return; }
    let Some(mut rect) = clipped(rect, display.width, display.height) else { return; };
    // Merge touching damage, retaining separate distant areas such as cursor/statusbar.
    let mut i = 0;
    while i < display.damage.len() {
        let other = display.damage[i];
        if rect[0] <= other[0] + other[2] && other[0] <= rect[0] + rect[2]
            && rect[1] <= other[1] + other[3] && other[1] <= rect[1] + rect[3] {
            let x = rect[0].min(other[0]);
            let y = rect[1].min(other[1]);
            rect = [x, y, (rect[0] + rect[2]).max(other[0] + other[2]) - x,
                (rect[1] + rect[3]).max(other[1] + other[3]) - y];
            display.damage.swap_remove(i);
            i = 0;
        } else { i += 1; }
    }
    if display.damage.len() >= 32 { display.dirty = true; }
    else { display.damage.push(rect); }
}
/// Copy the dirty rectangles of a software-renderer frame into the window's surface (R, G,
/// B, A); the next flush shows them.
pub fn win_present(id: i32, frame: &Frame<'_>, rects: Option<&[[i32; 4]]>) {
    let t0 = time_us();
    present_to(id, frame, rects);
    present_account(t0, true);
}
fn present_to(id: i32, frame: &Frame<'_>, rects: Option<&[[i32; 4]]>) {
    let Some((row, len)) = frame_size(frame) else { return; };
    let mut display = DISPLAY.lock();
    if let Some(window) = window_mut(&mut display, id) {
        let resized = window.width != frame.w || window.height != frame.h || window.pixels.len() != len;
        if resized {
            if window.pixels.try_reserve(len.saturating_sub(window.pixels.len())).is_err() { return; }
            window.pixels.resize(len, 0);
        }
        let all = [[0, 0, frame.w, frame.h]];
        let rects = if resized { &all[..] } else { rects.unwrap_or(&all) };
        let mut changed = Vec::new();
        for &rect in rects {
            let Some([x, y, w, h]) = clipped(rect, frame.w, frame.h) else { continue; };
            for yy in y..y + h {
                let src = yy as usize * frame.stride as usize + x as usize * 4;
                let dst = yy as usize * row + x as usize * 4;
                to_rgba(&mut window.pixels[dst..dst + w as usize * 4], &frame.px[src..src + w as usize * 4]);
            }
            changed.push([window.rect[0].saturating_add(x), window.rect[1].saturating_add(y), w, h]);
        }
        window.width = frame.w;
        window.height = frame.h;
        let full = resized || !window.shown;
        window.shown = true;
        if full { display.dirty = true; }
        for rect in changed { damage(&mut display, rect); }
    }
}
/// Request one of the shared browser pointer shapes.
pub fn win_set_cursor(_id: i32, shape: u32) {
    let changed = {
        let mut d = DISPLAY.lock();
        let changed = d.cursor != shape;
        d.cursor = shape;
        changed
    };
    if changed {
        super::notice(super::REQUEST_CURSOR, &shape.to_le_bytes());
    }
}
/// Hit testing remains inside the shared UI; no OS nonclient callback is installed.
pub fn win_hit_hook(_hook: Option<fn(i32, i32, i32) -> u32>) {}
/// Native modal resize callbacks are unnecessary for host-stepped browser rendering.
pub fn set_refresh(_callback: Option<fn()>) {}
/// Custom native cursor handles are unavailable; use the shared system-shape cursor mapping.
pub fn cursor_create(
    _pixels: &[u8],
    _width: i32,
    _height: i32,
    _x: i32,
    _y: i32,
    _dpi: i32,
) -> i32 {
    -1
}
/// No native custom cursor handle exists.
pub fn cursor_destroy(_id: i32) {}
/// Cursor scale follows the canvas DPI.
pub fn cursor_dpi(dpi: i32) -> i32 {
    dpi.clamp(48, 480)
}
/// The canvas shows popups where the library put them (inside the canvas).
pub fn popup_anchor(_slot: u32, _anchor: [i32; 4], _kind: u32, _nat: [i32; 2]) {}
/// Popups stay where the library put them.
pub fn popup_placed(_slot: u32) -> Option<[i32; 2]> {
    None
}
/// Paint an existing library popup into the canvas overlay stack.
pub fn popup(slot: u32, owner: i32, rect: Option<[i32; 4]>, frame: Option<&Frame<'_>>) {
    if slot >= PPW_MAX {
        return;
    }
    let value = rect.zip(frame).and_then(|(rect, frame)| {
        pixels(frame).map(|pixels| Popup {
            owner,
            rect,
            pixels,
            width: frame.w,
            height: frame.h,
        })
    });
    let mut d = DISPLAY.lock();
    if d.popups[slot as usize].is_none() && value.is_none() { return; }
    d.popups[slot as usize] = value;
    d.dirty = true;
}

fn overlay(
    output: &mut [u8],
    width: i32,
    height: i32,
    source: &[u8],
    sw: i32,
    sh: i32,
    x: i32,
    y: i32,
    clip: [i32; 4],
) {
    let left = x.max(clip[0]).max(0);
    let top = y.max(clip[1]).max(0);
    let right = x.saturating_add(sw).min(clip[0] + clip[2]).min(width);
    let bottom = y.saturating_add(sh).min(clip[1] + clip[3]).min(height);
    if right <= left || bottom <= top {
        return;
    }
    for row in top..bottom {
        let to = (row as usize * width as usize + left as usize) * 4;
        let from = ((row - y) as usize * sw as usize + (left - x) as usize) * 4;
        let len = (right - left) as usize * 4;
        if let (Some(dst), Some(src)) = (output.get_mut(to..to + len), source.get(from..from + len))
        {
            dst.copy_from_slice(src);
        }
    }
}
pub(super) fn flush() {
    let t0 = time_us();
    flush_to();
    present_account(t0, false);
}
fn flush_to() {
    let mut d = DISPLAY.lock();
    if !d.dirty && d.damage.is_empty() {
        return;
    }
    let (width, height) = (d.width, d.height);
    let Some(length) = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(4))
        .filter(|&v| v <= 256 << 20)
    else {
        return;
    };
    // one window over the whole canvas and no popup: its pixels are the canvas
    let mut shown = d.windows.iter().enumerate().filter(|(_, w)| w.as_ref().is_some_and(|w| w.shown));
    let single = match (shown.next(), shown.next()) {
        (Some((0, Some(w))), None) => {
            w.rect == [0, 0, width, height] && w.width == width && w.height == height && w.pixels.len() == length
        }
        _ => false,
    } && d.popups.iter().all(Option::is_none);
    if single {
        let mut rects = core::mem::take(&mut d.damage);
        if d.dirty {
            rects.clear();
            rects.push([0, 0, width, height]);
        }
        d.dirty = false;
        d.composed = false; // the next composition draws everything
        let Some(w) = window_mut(&mut d, 0) else { return };
        let px = core::mem::take(&mut w.pixels);
        drop(d);
        super::present_bytes(&px, width, height, width * 4, &rects);
        let mut d = DISPLAY.lock();
        if let Some(w) = window_mut(&mut d, 0) {
            w.pixels = px;
        }
        rects.clear();
        d.damage = rects;
        return;
    }
    let result = {
        let mut output = core::mem::take(&mut d.composite);
        if output.try_reserve(length.saturating_sub(output.len())).is_err() {
            d.composite = output;
            return;
        }
        if output.len() != length || !d.composed { d.dirty = true; }
        output.resize(length, 255);
        let mut rects = core::mem::take(&mut d.damage);
        if d.dirty { rects.clear(); rects.push([0, 0, width, height]); }
        for &clip in &rects {
          for y in clip[1]..clip[1] + clip[3] {
            let start = (y as usize * width as usize + clip[0] as usize) * 4;
            output[start..start + clip[2] as usize * 4].fill(255);
          }
          for window in d.windows.iter().flatten().filter(|w| w.shown) {
            overlay(
                &mut output,
                width,
                height,
                &window.pixels,
                window.width,
                window.height,
                window.rect[0],
                window.rect[1],
                clip,
            );
          }
          for popup in d.popups.iter().flatten() {
            let Some(owner) = window(&d, popup.owner).filter(|w| w.shown) else {
                continue;
            };
            overlay(
                &mut output,
                width,
                height,
                &popup.pixels,
                popup.width,
                popup.height,
                owner.rect[0].saturating_add(popup.rect[0]),
                owner.rect[1].saturating_add(popup.rect[1]),
                clip,
            );
          }
        }
        d.dirty = false;
        d.composed = true;
        (output, width, height, rects)
    };
    drop(d);
    super::present_bytes(&result.0, result.1, result.2, result.1 * 4, &result.3);
    let mut d = DISPLAY.lock();
    d.composite = result.0;
    d.damage = result.3;
    d.damage.clear();
}
pub(super) fn event(mut event: Event) -> bool {
    let mut d = DISPLAY.lock();
    match event.typ {
        E_RESIZE if event.win == 0 => {
            d.width = event.x.clamp(1, 16384);
            d.height = event.y.clamp(1, 16384);
            let (width, height) = (d.width, d.height);
            if let Some(w) = window_mut(&mut d, 0) {
                w.rect = [0, 0, width, height];
            }
            d.dirty = true;
        }
        E_DPI => {
            d.dpi = (event.data as i32).clamp(48, 480);
            let dpi = d.dpi;
            for w in d.windows.iter_mut().flatten() {
                w.dpi = dpi;
            }
        }
        E_MOVE | E_DOWN | E_UP | E_WHEEL => {
            d.pointer = (event.x, event.y);
            if let Some((id, sx, sy, rect, edges)) = d.drag {
                if event.typ == E_MOVE {
                    if let Some(w) = window_mut(&mut d, id) {
                        let (dx, dy) = (event.x - sx, event.y - sy);
                        if edges == 0 {
                            w.rect[0] = rect[0] + dx;
                            w.rect[1] = rect[1] + dy;
                        } else {
                            let minw = w.min.0 * w.dpi / 96;
                            let minh = w.min.1 * w.dpi / 96;
                            if edges & EDGE_RIGHT != 0 {
                                w.rect[2] = (rect[2] + dx).clamp(minw, 16384);
                            }
                            if edges & EDGE_BOTTOM != 0 {
                                w.rect[3] = (rect[3] + dy).clamp(minh, 16384);
                            }
                            if edges & EDGE_LEFT != 0 {
                                let width = (rect[2] - dx).clamp(minw, 16384);
                                w.rect[0] = rect[0] + rect[2] - width;
                                w.rect[2] = width;
                            }
                            if edges & EDGE_TOP != 0 {
                                let height = (rect[3] - dy).clamp(minh, 16384);
                                w.rect[1] = rect[1] + rect[3] - height;
                                w.rect[3] = height;
                            }
                        }
                        let _ = evq_push(Event {
                            typ: E_RESIZE,
                            win: id as u8,
                            x: w.rect[2],
                            y: w.rect[3],
                            ..Event::default()
                        });
                    }
                    d.dirty = true;
                }
                if event.typ == E_UP {
                    d.drag = None;
                }
            }
            let target = d.capture.unwrap_or_else(|| {
                if let Some(popup) = d.popups.iter().rev().flatten().find(|p| {
                    window(&d, p.owner).is_some_and(|w| {
                        let x = w.rect[0].saturating_add(p.rect[0]);
                        let y = w.rect[1].saturating_add(p.rect[1]);
                        w.shown
                            && w.enabled
                            && event.x >= x
                            && event.y >= y
                            && event.x < x.saturating_add(p.rect[2])
                            && event.y < y.saturating_add(p.rect[3])
                    })
                }) {
                    return popup.owner;
                }
                d.windows
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, w)| {
                        w.as_ref().is_some_and(|w| {
                            w.shown
                                && w.enabled
                                && event.x >= w.rect[0]
                                && event.y >= w.rect[1]
                                && event.x < w.rect[0] + w.rect[2]
                                && event.y < w.rect[1] + w.rect[3]
                        })
                    })
                    .map_or(d.active, |(i, _)| i as i32)
            });
            if event.typ == E_DOWN {
                d.capture = Some(target);
                d.active = target;
            }
            if event.typ == E_UP {
                d.capture = None;
            }
            if let Some(w) = window(&d, target) {
                if !w.enabled {
                    return false;
                }
                event.x -= w.rect[0];
                event.y -= w.rect[1];
                event.win = target as u8;
            }
        }
        E_KEY_DOWN | E_KEY_UP | E_TEXT => {
            if !window(&d, d.active).is_some_and(|w| w.shown && w.enabled) {
                return false;
            }
            event.win = d.active as u8;
        }
        _ => {}
    }
    drop(d);
    evq_push(event)
}
