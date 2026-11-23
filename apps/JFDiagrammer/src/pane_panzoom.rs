// pane_panzoom -- (JD8 rest) the task pane Verschieben und Zoomen (AB-06): the page shown small with
// the part the canvas shows as a frame, and a zoom slider
//
//   pane(app, ui, id)        the overview and the slider
//
// The overview is the page drawn the way the canvas draws it (PagePaint::thumbnail, kept while the
// page and its edits stay the same).  On it:
//   - dragging inside the frame moves the view along (pan);
//   - (JD15) dragging an edge or a corner of the frame (EDGE_DIP around it) changes the frame's
//     size, and the canvas zooms to the new frame as it goes (Visio's);
//   - dragging elsewhere draws a rectangle the canvas then zooms to; a click centres the view there;
//   - the wheel zooms in and out around the frame's middle.

use alloc::rc::Rc;

use gfx::Surface;
use ui::i18n::t;
use ui::*;

use crate::viewer::{self, DIP};
use crate::Diagrammer;

/// the overview's picture and the drag in progress
#[derive(Default)]
pub(crate) struct PanZoom {
    thumb: Option<(u32, i32, i32, Rc<Surface>)>,
    drag: Option<Drag>,
    /// the frame as drawn last (overview px x, y, w, h): the size pointer over its edges
    frame: Option<[i32; 4]>,
}

#[derive(Clone, Copy)]
enum Drag {
    /// the frame grabbed: the pointer's offset from the view's middle (overview px)
    Pan([f32; 2]),
    /// a rectangle from this point (overview px)
    Rect([i32; 2]),
    /// (JD15) edges of the frame grabbed (1 left, 2 top, 4 right, 8 bottom) and the frame at
    /// the press (overview px x0, y0, x1, y1)
    Edge(u8, [f32; 4]),
}

const AREA_H: i32 = 180;
// how near to an edge of the frame a press grabs it (DIP)
const EDGE_DIP: i32 = 4;

// the edges of `frame` (x, y, w, h) a pointer at (mx, my) is on: 1 left, 2 top, 4 right, 8 bottom
// (a side shorter than three times the reach has none, so a small frame can still be dragged)
fn edges_at(frame: [i32; 4], mx: i32, my: i32, tol: i32) -> u8 {
    let (x0, y0, x1, y1) = (frame[0], frame[1], frame[0] + frame[2], frame[1] + frame[3]);
    if mx < x0 - tol || mx > x1 + tol || my < y0 - tol || my > y1 + tol {
        return 0;
    }
    let mut e = 0;
    if frame[2] >= 3 * tol {
        e |= if (mx - x0).abs() <= tol { 1 } else if (mx - x1).abs() <= tol { 4 } else { 0 };
    }
    if frame[3] >= 3 * tol {
        e |= if (my - y0).abs() <= tol { 2 } else if (my - y1).abs() <= tol { 8 } else { 0 };
    }
    e
}

// the size pointer for edges
fn edge_cursor(e: u8) -> u32 {
    match e {
        1 | 4 => CUR_SIZE_WE,
        2 | 8 => CUR_SIZE_NS,
        3 | 12 => CUR_SIZE_NWSE,
        _ => CUR_SIZE_NESW,
    }
}

/// the pane
pub(crate) fn pane(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    ui.next_size(-1, -1);
    ui.box_begin(id, BOX_V | pad(10), 8);
    let k = app.page_index();
    let paint = viewer::paint(app, k);
    let ver = app.opened.as_ref().map_or(0, |o| viewer::canvas_ver(o.serial, k, o.ed.rev));
    // (JD15) over an edge of the frame (or dragging one): the size pointer
    let tol = ui.dp(EDGE_DIP);
    let (mx, my) = ui.mouse();
    let hot = match app.panzoom.drag {
        Some(Drag::Edge(e, _)) => e,
        Some(_) => 0,
        None => app.panzoom.frame.map_or(0, |f| edges_at(f, mx, my, tol)),
    };
    if hot != 0 {
        ui.cursor_next(edge_cursor(hot));
    }
    ui.next_size(-1, AREA_H);
    let inp = ui.canvas(id + 1, None, 0, ver).clone();
    let r = inp.rect;
    if let (Some(p), true) = (paint, r[2] > 16 && r[3] > 16) {
        let (pw, ph) = p.size();
        // the picture: the page's proportions inside the area
        let (w, h) = if pw * r[3] as f64 > ph * r[2] as f64 { (r[2], ((r[2] as f64) * ph / pw) as i32) } else { (((r[3] as f64) * pw / ph) as i32, r[3]) };
        let (w, h) = (w.max(8), h.max(8));
        let fresh = app.panzoom.thumb.as_ref().is_some_and(|t| t.0 == ver && t.1 == w && t.2 == h);
        if !fresh {
            if let Some(sf) = p.thumbnail(w as u32, h as u32) {
                app.panzoom.thumb = Some((ver, w, h, Rc::new(sf)));
            }
        }
        let ox = r[0] + (r[2] - w) / 2;
        let oy = r[1] + (r[3] - h) / 2;
        if let Some((_, _, _, sf)) = app.panzoom.thumb.as_ref() {
            ui.draw_image(sf, ver, ox, oy, w, h);
        }
        // paper inches -> overview px, as PageView::render fits the page (2 % margin)
        let m = ((w.min(h) as f64) * 0.02).max(2.0);
        let s = ((w as f64 - 2.0 * m) / pw).min((h as f64 - 2.0 * m) / ph);
        let (px0, py0) = (ox as f64 + (w as f64 - pw * s) * 0.5, oy as f64 + (h as f64 - ph * s) * 0.5);
        // world (DIP, y down from the page's top) <-> overview px
        let to_px = |wx: f32, wy: f32| [(px0 + wx as f64 / DIP * s) as f32, (py0 + wy as f64 / DIP * s) as f32];
        let to_world = |x: f32, y: f32| [((x as f64 - px0) / s * DIP) as f32, ((y as f64 - py0) / s * DIP) as f32];
        let view = app.canvas.vp.world_rect();
        let a = to_px(view[0], view[1]);
        let b = to_px(view[2], view[3]);
        let accent = ui.color(T_ACCENT);
        let frame = [a[0] as i32, a[1] as i32, (b[0] - a[0]).max(2.0) as i32, (b[1] - a[1]).max(2.0) as i32];
        app.panzoom.frame = Some(frame);
        ui.clip_push(r[0], r[1], r[2], r[3]);
        ui.cmd_rrect(frame[0], frame[1], frame[2], frame[3], 0, ui.dp(2), (accent & 0x00FF_FFFF) | 0x2000_0000, accent, accent);
        // the pointer, absolute
        let (mx, my) = (r[0] + inp.x, r[1] + inp.y);
        let left = 1 << BTN_LEFT;
        if inp.pressed & left != 0 && inp.ev & EV_HOVER != 0 {
            let inside = mx >= frame[0] && mx < frame[0] + frame[2] && my >= frame[1] && my < frame[1] + frame[3];
            let c = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let e = edges_at(frame, mx, my, tol);
            app.panzoom.drag = Some(if e != 0 {
                Drag::Edge(e, [a[0], a[1], b[0], b[1]])
            } else if inside {
                Drag::Pan([mx as f32 - c[0], my as f32 - c[1]])
            } else {
                Drag::Rect([mx, my])
            });
        }
        match app.panzoom.drag {
            // (JD15) an edge: the frame from the press with the grabbed edges at the pointer; the
            // canvas shows it (its middle, the zoom that fits it in)
            Some(Drag::Edge(e, f0)) if inp.btn & left != 0 => {
                let (px, py) = (mx as f32, my as f32);
                let mut f = f0;
                let min = 4.0;
                if e & 1 != 0 {
                    f[0] = px.min(f[2] - min);
                }
                if e & 4 != 0 {
                    f[2] = px.max(f[0] + min);
                }
                if e & 2 != 0 {
                    f[1] = py.min(f[3] - min);
                }
                if e & 8 != 0 {
                    f[3] = py.max(f[1] + min);
                }
                let a0 = to_world(f[0], f[1]);
                let b0 = to_world(f[2], f[3]);
                let vp = &mut app.canvas.vp;
                let dpi = vp.dpi.max(1) as f32 / 96.0;
                let z = (vp.rect[2] as f32 / ((b0[0] - a0[0]) * dpi)).min(vp.rect[3] as f32 / ((b0[1] - a0[1]) * dpi));
                if z.is_finite() && z > 0.0 {
                    let z = z.clamp(ui::Viewport::ZOOM_MIN, ui::Viewport::ZOOM_MAX);
                    let (cx, cy) = (((a0[0] + b0[0]) * 0.5) as f64, ((a0[1] + b0[1]) * 0.5) as f64);
                    if (z, cx, cy) != (vp.zoom, vp.cx, vp.cy) {
                        vp.zoom = z;
                        vp.cx = cx;
                        vp.cy = cy;
                        ui.repass();
                    }
                }
            }
            Some(Drag::Pan(off)) if inp.btn & left != 0 => {
                let w0 = to_world(mx as f32 - off[0], my as f32 - off[1]);
                app.canvas.vp.cx = w0[0] as f64;
                app.canvas.vp.cy = w0[1] as f64;
                ui.repass();
            }
            Some(Drag::Rect(st)) if inp.btn & left != 0 => {
                let (x0, y0) = (st[0].min(mx), st[1].min(my));
                ui.cmd_rrect(x0, y0, (st[0] - mx).abs().max(1), (st[1] - my).abs().max(1), 0, ui.dp(1), (accent & 0x00FF_FFFF) | 0x3000_0000, accent, accent);
            }
            Some(Drag::Rect(st)) if inp.released & left != 0 => {
                app.panzoom.drag = None;
                let a0 = to_world(st[0].min(mx) as f32, st[1].min(my) as f32);
                let b0 = to_world(st[0].max(mx) as f32, st[1].max(my) as f32);
                let vp = &mut app.canvas.vp;
                vp.cx = ((a0[0] + b0[0]) * 0.5) as f64;
                vp.cy = ((a0[1] + b0[1]) * 0.5) as f64;
                // a rectangle (not a click): zoom so that it fills the canvas
                if (st[0] - mx).abs() > 4 && (st[1] - my).abs() > 4 {
                    let dpi = vp.dpi.max(1) as f32 / 96.0;
                    let z = (vp.rect[2] as f32 / ((b0[0] - a0[0]) * dpi)).min(vp.rect[3] as f32 / ((b0[1] - a0[1]) * dpi));
                    if z.is_finite() && z > 0.0 {
                        vp.zoom = z.clamp(ui::Viewport::ZOOM_MIN, ui::Viewport::ZOOM_MAX);
                    }
                }
                ui.repass();
            }
            Some(_) if inp.btn & left == 0 => app.panzoom.drag = None,
            _ => {}
        }
        if inp.wheel_y != 0 && inp.ev & EV_HOVER != 0 {
            app.zoom_step(if inp.wheel_y > 0 { 1 } else { -1 });
            ui.repass();
        }
        ui.clip_pop();
        ui.tooltip(t("Ziehen verschiebt die Ansicht, ein Rechteck zoomt dorthin."));
    }
    // (JD15) the status bar's slider: 100 % in the middle
    ui.next_size(-1, 0);
    crate::panes::zoom_slider_here(app, ui, id + 2);
    ui.box_end();
}
