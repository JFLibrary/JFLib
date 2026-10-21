// canvas -- the topology: ui::diagram over the project's devices, cables and notes, its events
// (move, delete, place, rename, connect, waypoints, context menus), drops of palette tiles, the
// rename flyout
//
//   Canvas                      the diagram state (view, selection, grid), the scene, the tool,
//                               the move in progress, the rename flyout, the cable being drawn,
//                               the port choice, the waypoint drag, the × of the hovered cable
//   frame(n, ui)                the canvas in the dock's centre (info bar, diagram, minimap)
//   canvas.reset(&project)      after open / new: no selection, the saved view (or fit)
//   canvas.store_view(&mut p)   the view into the project's layout (before saving)
//   canvas.open_rename(p, id)   the flyout under a device (its name) or a note (its text)
//   select_all / delete_selection / zoom(n, k)    the commands of the menus
//
// Diagram keys are the project's ids (devices, links and notes share the id counter; ports
// have keys of their own, paint::port_key).  A drag moves the project's positions live and
// becomes one undo step when it ends; arrow keys are a step each.  The names follow
// jfn_model's rules; a refused name keeps the flyout open with the reason below the field.  A
// tile armed for a click or dragged over the canvas shows its device half transparent where it
// would land.  A double click (or Enter) opens a device's console, with Shift its device window
// (workspace/terminal.rs, devwin/); a note's text opens in the flyout.  connect.rs joins
// devices (tool C, port dots, the port choice), cables.rs edits cables (route, kind,
// waypoints, the ×, tooltips); the LEDs come from the simulator.

pub(crate) mod cables;
pub(crate) mod connect;
pub(crate) mod paint;
pub(crate) mod tools;

use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::{catalog, Link, Project};
use ui::*;

use crate::app::commands::{M_CABLE, M_CANVAS, M_DEVICE};
use crate::app::constants::*;
use crate::app::Net;
use crate::doc::edits;
use crate::workspace::icons::{tint_of, Symbols};
use paint::{Scene, DEV_H, DEV_W};
use tools::Tool;

/// the rename flyout: a device's name or a note's text
pub(crate) struct Rename {
    pub id: u32,
    pub note: bool,
    pub text: String,
    /// why the last name was refused
    pub err: String,
    opened: bool,
    select: bool,
}

/// a view to take after a load (applied after the next diagram call)
enum View {
    At(f32, f64, f64),
    Fit,
}

pub(crate) struct Canvas {
    pub st: DiagramState,
    pub scene: Scene,
    pub painter: Option<Rc<dyn DgPaint>>,
    pub tool: Tool,
    /// the start positions of the things a drag moves
    drag: Option<Vec<(u32, [i32; 2])>>,
    pub rename: Option<Rename>,
    view: Option<View>,
    /// the cable being drawn (connect.rs)
    pub conn: Option<connect::Drag>,
    /// the port choice after a cable was dropped (connect.rs)
    pub choice: Option<connect::Choice>,
    /// the cable as it was before a waypoint drag (cables.rs)
    pub wp_drag: Option<Link>,
    /// the cable with the × (cables.rs)
    pub x_link: Option<u32>,
}

impl Canvas {
    pub(crate) fn new() -> Canvas {
        Canvas {
            st: DiagramState::new(),
            scene: Scene::default(),
            painter: None,
            tool: Tool::Select,
            drag: None,
            rename: None,
            view: None,
            conn: None,
            choice: None,
            wp_drag: None,
            x_link: None,
        }
    }

    /// a new project: nothing selected, its view (a file without one: everything shown)
    pub(crate) fn reset(&mut self, p: &Project) {
        self.st.sel.clear();
        self.st.focus = 0;
        self.drag = None;
        self.rename = None;
        self.conn = None;
        self.choice = None;
        self.wp_drag = None;
        self.x_link = None;
        let l = &p.layout;
        self.view = Some(if l.zoom_pm > 0 { View::At(l.zoom_pm as f32 / 1000.0, l.cx as f64, l.cy as f64) } else { View::Fit });
    }

    /// the view into the project's layout
    pub(crate) fn store_view(&self, p: &mut Project) {
        let vp = &self.st.vp;
        p.layout.zoom_pm = (vp.zoom * 1000.0 + 0.5) as u32;
        p.layout.cx = round(vp.cx);
        p.layout.cy = round(vp.cy);
    }

    pub(crate) fn select_only(&mut self, id: u32) {
        self.st.sel.clear();
        self.st.sel.insert(id as u64);
        self.st.focus = id as u64;
    }

    /// the selected ids in order
    pub(crate) fn selected(&self) -> Vec<u32> {
        self.st.sel.iter().map(|&k| k as u32).collect()
    }

    /// open the flyout for a device's name or a note's text
    pub(crate) fn open_rename(&mut self, p: &Project, id: u32) {
        let (note, text) = match edits::item(p, id) {
            Some(edits::Item::Device(_)) => (false, p.device(id).map(|d| d.name.clone()).unwrap_or_default()),
            Some(edits::Item::Note(_)) => (true, p.note(id).map(|n| n.text.clone()).unwrap_or_default()),
            Some(edits::Item::Link(_)) | None => return,
        };
        self.rename = Some(Rename { id, note, text, err: String::new(), opened: false, select: true });
    }
}

fn round(v: f64) -> i32 {
    let f = v + 0.5;
    let t = f as i32;
    if (t as f64) > f { t - 1 } else { t }
}

/// the canvas in the dock's centre
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    // the scene follows the project
    if n.canvas.scene.rev != n.doc.rev || n.canvas.scene.ver == 0 {
        n.canvas.scene.build(&n.doc.proj, &n.syms, n.doc.rev);
    }
    n.canvas.scene.grid = n.prefs.grid;
    let g = n.doc.proj.layout.grid;
    n.canvas.st.grid = if g == 0 { 20.0 } else { g.clamp(4, 200) as f32 };
    n.canvas.st.snap = n.prefs.snap;
    // Esc gives up an armed tile before the canvas takes it for its selection
    if matches!(n.canvas.tool, Tool::Place(_) | Tool::Note | Tool::Pdu) && ui.popup_count() == 0 && ui.modal_active() == 0 && ui.key(VK_ESCAPE) {
        n.canvas.tool = Tool::Select;
    }
    infobar(n, ui);
    let minimap = n.prefs.minimap;
    // the painting depends on the symbols' palettes, the grid and the simulator's LEDs
    let ver = n.syms.key() ^ if n.prefs.grid { 0 } else { 0x8000_0000 } ^ n.doc.sim.led_key.rotate_left(7);
    let tool = n.canvas.tool.dg();
    let dragging = n.palette.dragging;
    ui.next_size(-1, -1);
    let (evs, dropped, preview) = {
        let doc = &n.doc;
        let c = &mut n.canvas;
        let m = c.scene.model();
        let evs = ui.diagram(ID_CANVAS, &mut c.st, &m, tool, c.painter.clone(), ver);
        // the canvas' tooltips (right after it, the last widget): the target that does not
        // fit, else the cable or port dot under the pointer
        if !connect::tooltip(c, ui) {
            cables::tooltip(c, doc, ui);
        }
        let hovered = ui.last_state() & SS_HOVER as u32 != 0;
        let target = ui.drop_target(ID_CANVAS, DT_DEVICE);
        let mut dropped = None;
        if target == DS_DROP {
            let d = ui.drop_data();
            let id = String::from_utf8_lossy(&d.bytes).into_owned();
            let (x, y) = ui.drop_pos();
            let at = c.st.vp.to_world(ui.dp(x) as f32 + 0.5, ui.dp(y) as f32 + 0.5);
            dropped = catalog::catalog().iter().position(|m| m.id == id).map(|i| (i, at));
        }
        // the pointer of a tile dragged over the canvas, or armed for a click (px)
        let preview = match (dragging, c.tool) {
            (Some(i), _) if target == DS_HOVER => {
                let (_, x, y) = ui.drag_over();
                Some((i, ui.dp(x), ui.dp(y), false))
            }
            (None, Tool::Place(i)) if hovered && ui.popup_count() == 0 => {
                let (x, y) = ui.mouse();
                Some((i, x, y, true))
            }
            _ => None,
        };
        if minimap {
            ui.diagram_minimap(ID_MINIMAP, &mut c.st, &m);
        }
        let sc = &c.scene;
        ui.diagram_sems(ID_CANVAS, &mut c.st, &m, &|k| sc.label(k));
        (evs, dropped, preview)
    };
    if let Some((model, x, y, click)) = preview {
        preview_device(n, ui, model, x, y, click);
    }
    cables::leds_on_top(n, ui);
    cables::plus_handles(n, ui);
    connect::overlay(n, ui);
    cables::hover_x(n, ui);
    // the view of a project that was just loaded (once the canvas has its size: the widget's
    // first fit happens then and must not undo it)
    let sized = n.canvas.st.vp.rect[2] > 0 && n.canvas.st.vp.rect[3] > 0;
    if let Some(v) = if sized { n.canvas.view.take() } else { None } {
        match v {
            View::At(z, x, y) => {
                n.canvas.st.vp.zoom = z.clamp(Viewport::ZOOM_MIN, Viewport::ZOOM_MAX);
                n.canvas.st.vp.cx = x;
                n.canvas.st.vp.cy = y;
            }
            View::Fit => n.canvas.st.request_fit(),
        }
        ui.repass();
    }
    for ev in evs {
        event(n, ui, ev);
    }
    if let Some((model, at)) = dropped {
        tools::place(n, ui, model, at);
        ui.repass();
    }
    rename_flyout(n, ui);
    connect::popover(n, ui);
}

/// a device a tile would place, half transparent where it would land
struct Ghost {
    syms: Rc<Symbols>,
    sym: usize,
    tint: usize,
}

impl Draw for Ghost {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let h = self.syms.height(self.sym, r[2]);
        self.syms.draw_as(g, self.sym, r[0], r[1] + (r[3] - h) / 2, r[2], &self.syms.pal(self.tint), 128);
    }
}

fn fl(v: f32) -> i32 {
    let t = v as i32;
    if (t as f32) > v { t - 1 } else { t }
}

// the preview of catalog model `model` for the pointer at px x, y (click: snapped as a
// placing click is); notes have none
fn preview_device(n: &Net, ui: &mut Ui, model: usize, x: i32, y: i32, click: bool) {
    let Some(m) = catalog::catalog().get(model) else { return };
    if m.has(catalog::F_ANNOTATION) {
        return;
    }
    let Some(sym) = n.syms.index(m.symbol) else { return };
    let vp = n.canvas.st.vp;
    let mut at = vp.to_world(x as f32 + 0.5, y as f32 + 0.5);
    if click {
        at = tools::click_point(n, at);
    }
    let o = tools::origin(n, at);
    let q = vp.rect_to_screen([o[0] as f32, o[1] as f32, DEV_W, DEV_H]);
    let (px, py) = (fl(q[0] + 0.5), fl(q[1] + 0.5));
    let pw = (fl(q[0] + q[2] + 0.5) - px).max(1);
    let ph = (fl(q[1] + q[3] + 0.5) - py).max(1);
    let d: DrawRef = Rc::new(Ghost { syms: n.syms.clone(), sym, tint: tint_of(m.category) });
    let ver = n.syms.key() ^ (sym as u32).wrapping_mul(0x9E37_79B9) ^ (pw as u32) << 20;
    let r = vp.rect;
    ui.clip_push(r[0], r[1], r[2], r[3]);
    ui.cmd_custom(px, py, pw, ph, Some(d), 0, ver);
    ui.clip_pop();
}

fn infobar(n: &mut Net, ui: &mut Ui) {
    let Some((sev, title, msg)) = n.file.msg.clone() else { return };
    ui.box_begin(0, BOX_V | pad(8), 0);
    if ui.infobar(ID_INFOBAR, sev, &title, &msg) & EV_ACT != 0 {
        n.file.msg = None;
        ui.repass();
    }
    ui.box_end();
}

fn event(n: &mut Net, ui: &mut Ui, ev: DEv) {
    match ev {
        DEv::Move { keys, total, done, .. } => move_items(n, &keys, total, done),
        DEv::Delete => delete_selection(n, ui),
        DEv::Click { at } => {
            let keep = ui.mods() & MOD_SHIFT != 0;
            match n.canvas.tool {
                Tool::Place(m) => {
                    tools::place(n, ui, m, at);
                    if !keep {
                        n.canvas.tool = Tool::Select;
                    }
                }
                Tool::Note => {
                    tools::place_note(n, ui, at);
                    if !keep {
                        n.canvas.tool = Tool::Select;
                    }
                }
                Tool::Pdu => crate::workspace::sim::pdu::pick(n, ui),
                _ => {}
            }
        }
        // a device: its console (Shift: its device window), a note: its text
        DEv::Activate(DHit::Node(k)) => crate::workspace::terminal::activate(n, ui, k as u32),
        DEv::Context(hit, _) => {
            let menu: &'static [Mi] = match hit {
                DHit::Edge { .. } | DHit::Waypoint(..) | DHit::Mid(..) => &M_CABLE,
                DHit::Node(k) if n.doc.proj.device(k as u32).is_some() => &M_DEVICE,
                DHit::Port(k) => {
                    // a port dot: its device
                    if let Some((d, _)) = paint::key_port(k) {
                        n.canvas.select_only(d);
                    }
                    &M_DEVICE
                }
                _ => &M_CANVAS,
            };
            ui.menu_popup(menu);
        }
        DEv::ConnectDrag { from, over, .. } => connect::drag(n, from, over),
        DEv::Connect { from, to, quick } => connect::finish(n, ui, from, to, quick),
        DEv::ConnectCancel => n.canvas.conn = None,
        DEv::Waypoint { edge, idx, at, insert, done } => cables::waypoint(n, edge, idx, at, insert, done),
        DEv::WaypointDelete { edge, idx } => cables::waypoint_delete(n, edge, idx),
        _ => {}
    }
}

// round to whole world units
fn rnd(v: f32) -> i32 {
    let f = v + 0.5;
    let t = f as i32;
    if (t as f32) > f { t - 1 } else { t }
}

/// a drag (or an arrow key) moves devices and notes: live in the project, one undo step at
/// its end
fn move_items(n: &mut Net, keys: &[u64], total: [f32; 2], done: bool) {
    let from = match n.canvas.drag.take() {
        Some(f) => f,
        None => keys.iter().filter_map(|&k| edits::pos(&n.doc.proj, k as u32).map(|p| (k as u32, p))).collect(),
    };
    let d = [rnd(total[0]), rnd(total[1])];
    for &(id, p) in &from {
        edits::set_pos(&mut n.doc.proj, id, [p[0] + d[0], p[1] + d[1]]);
    }
    n.doc.touch();
    if done {
        if let Some(e) = edits::moves(&n.doc.proj, &from) {
            n.doc.record(e);
        }
    } else {
        n.canvas.drag = Some(from);
    }
}

/// delete the selected devices (with their links) and notes
pub(crate) fn delete_selection(n: &mut Net, ui: &mut Ui) {
    let ids = n.canvas.selected();
    if let Some(e) = edits::delete(&n.doc.proj, &ids) {
        n.doc.apply(e);
    }
    n.canvas.st.sel.clear();
    n.canvas.st.focus = 0;
    ui.repass();
}

/// the ids of every device, cable and note
pub(crate) fn ids(p: &Project) -> BTreeSet<u32> {
    p.devices.iter().map(|d| d.id).chain(p.links.iter().map(|l| l.id)).chain(p.notes.iter().map(|o| o.id)).collect()
}

/// select every device, cable and note
pub(crate) fn select_all(n: &mut Net, ui: &mut Ui) {
    let st = &mut n.canvas.st;
    st.sel.clear();
    for d in &n.doc.proj.devices {
        st.sel.insert(d.id as u64);
    }
    for l in &n.doc.proj.links {
        st.sel.insert(l.id as u64);
    }
    for o in &n.doc.proj.notes {
        st.sel.insert(o.id as u64);
    }
    ui.repass();
}

/// zoom one step in (1) or out (-1) around the centre, 100 % (0), everything (2)
pub(crate) fn zoom(n: &mut Net, ui: &mut Ui, k: i32) {
    let vp = &mut n.canvas.st.vp;
    let r = vp.rect;
    let (cx, cy) = (r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
    match k {
        1 | -1 => vp.step(k, cx, cy),
        0 => vp.zoom_at(1.0, cx, cy),
        // the widget's own fit (Ctrl+0 on the canvas does the same)
        _ => n.canvas.st.request_fit(),
    }
    ui.repass();
}

/// the selected device or note, when exactly one is selected (what F2 renames)
pub(crate) fn single(n: &Net) -> Option<u32> {
    match single_item(n)? {
        edits::Item::Device(id) | edits::Item::Note(id) => Some(id),
        edits::Item::Link(_) => None,
    }
}

/// the selected device, note or cable, when exactly one is selected
pub(crate) fn single_item(n: &Net) -> Option<edits::Item> {
    if n.canvas.st.sel.len() != 1 {
        return None;
    }
    let id = *n.canvas.st.sel.iter().next()? as u32;
    edits::item(&n.doc.proj, id)
}

/// F2: rename the selected device (or edit the selected note)
pub(crate) fn rename_selection(n: &mut Net, ui: &mut Ui) {
    if let Some(id) = single(n) {
        n.canvas.open_rename(&n.doc.proj, id);
        ui.repass();
    }
}

// the flyout under the device or note being renamed
fn rename_flyout(n: &mut Net, ui: &mut Ui) {
    if n.canvas.rename.is_none() && ui.popup_is_open(ID_RENAME) {
        ui.popup_close(); // given up elsewhere (undo, another project)
    }
    let mut gone = false;
    if let Some(r) = &mut n.canvas.rename {
        if !r.opened {
            r.opened = true;
            ui.popup_open(ID_RENAME);
            ui.focus(ID_RENAME_BOX);
        } else if !ui.popup_is_open(ID_RENAME) {
            gone = true; // Esc, a click elsewhere
        }
    }
    if gone || n.canvas.rename.as_ref().is_some_and(|r| edits::item(&n.doc.proj, r.id).is_none()) {
        if ui.popup_is_open(ID_RENAME) {
            ui.popup_close();
        }
        n.canvas.rename = None;
    }
    // the anchor: the item's box on the screen, kept inside the canvas
    if let Some(rect) = n.canvas.rename.as_ref().and_then(|r| n.canvas.scene.rect(r.id as u64)) {
        let vp = &n.canvas.st.vp;
        let q = vp.rect_to_screen(rect);
        let c = vp.rect;
        let x0 = (q[0] as i32).clamp(c[0], c[0] + c[2] - 1);
        let x1 = ((q[0] + q[2]) as i32).clamp(x0 + 1, c[0] + c[2]);
        let y1 = ((q[1] + q[3]) as i32 + ui.dp(20)).clamp(c[1] + 1, c[1] + c[3]);
        let y0 = (q[1] as i32).clamp(c[1], y1 - 1);
        ui.last_set([x0, y0, x1 - x0, y1 - y0], 0);
    }
    let mut act = 0; // 1 commit, 2 cancel
    if ui.flyout_begin(ID_RENAME, FL_BELOW) {
        if let Some(r) = &mut n.canvas.rename {
            ui.box_begin(0, BOX_V | pad(8), 6);
            ui.text(TS_BODY_STRONG, if r.note { "Notiz" } else { "Gerätename" });
            // Esc gives up (the text box would only drop the focus)
            if ui.focused() == ID_RENAME_BOX && ui.capture_shortcut(VK_ESCAPE, 0) {
                act = 2;
            }
            let ev = if r.note {
                ui.next_size(280, 110);
                ui.textbox(ID_RENAME_BOX, Some("Text der Notiz"), &mut r.text, 2000 | TB_MULTI | TB_WRAP)
            } else {
                ui.next_size(240, 0);
                ui.textbox(ID_RENAME_BOX, Some("Name"), &mut r.text, 64)
            };
            if r.select && ui.textbox_select(ID_RENAME_BOX, &r.text, 0, r.text.len()) {
                r.select = false;
            }
            if ev & EV_ACT != 0 {
                r.err.clear();
            }
            if !r.note && ev & EV_COMMIT != 0 && ui.focused() == ID_RENAME_BOX {
                act = 1; // Enter
            }
            if !r.err.is_empty() {
                crate::workspace::error_text(ui, &r.err, 240);
            } else if !r.note {
                ui.next_size(240, 0);
                ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, "Buchstaben, Ziffern und Bindestriche, vorn ein Buchstabe");
            }
            ui.box_begin(0, BOX_H | BOX_END, 8);
            if ui.accent_button(ID_RENAME_OK, if r.note { "Übernehmen" } else { "Umbenennen" }) & EV_ACT != 0 {
                act = 1;
            }
            if ui.button(ID_RENAME_CANCEL, "Abbrechen") & EV_ACT != 0 {
                act = 2;
            }
            ui.box_end();
            ui.box_end();
        }
    }
    ui.flyout_end();
    match act {
        1 => commit_rename(n, ui),
        2 => {
            ui.popup_close(); // the keyboard goes back where it was
            n.canvas.rename = None;
        }
        _ => {}
    }
}

fn commit_rename(n: &mut Net, ui: &mut Ui) {
    let Some(r) = &mut n.canvas.rename else { return };
    let id = r.id;
    if r.note {
        let text = String::from(r.text.trim_end());
        if let Some(e) = edits::note_text(&n.doc.proj, id, &text) {
            n.doc.apply(e);
        }
    } else {
        match edits::rename(&mut n.doc.proj, id, &r.text) {
            Ok(Some(e)) => n.doc.record(e),
            Ok(None) => {}
            Err(e) => {
                r.err = alloc::format!("{}", e);
                ui.repass();
                return;
            }
        }
    }
    ui.popup_close();
    n.canvas.rename = None;
    ui.repass();
}
