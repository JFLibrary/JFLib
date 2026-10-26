// phys -- the physical view (P1), the second view of the workspace beside the logical canvas:
// the project's places as a hierarchy to walk, a room as a floor plan with its racks and desks,
// a rack in front view with its devices in their slots, the cables between places with their
// lengths, the ones over their limit marked
//
//   Phys                     the view: shown or not, the place shown, the way back, the
//                            diagram's state, the scene, the selected place, a drag in progress,
//                            the rename flyout, the last refusal, the inspector's edit buffers
//   frame(n, ui)             the view in the dock's centre (instead of the logical canvas):
//                            the bar (Zurück, Nach oben, the breadcrumbs Standort › Gebäude ›
//                            Raum, adding, renaming and removing places), a hint or a refusal,
//                            the plan (ui::diagram)
//   toolbar(n, ui)           the switch Logisch | Physisch of the toolbar
//   command(n, ui, id, arg)  the view's commands (Ansicht › Logische / Physische Ansicht with
//                            Ctrl+Shift+L / Ctrl+Shift+P, Ansicht › Physischer Plan ›); while
//                            it is shown also zoom, Alles zeigen, Löschen, Umbenennen, Alles
//                            auswählen and Ctrl+9 (None: not the view's)
//   states(n, ui)            enabled / checked of those commands
//   reset(&project) / store_view(&mut project)   the view of a loaded project and into its
//                            layout before saving (Layout::phys, phys_at)
//   show_device(n, id)       the plan of the place a device stands in, the device selected
//
// Navigation: a double click or Enter on a place opens it (a desk: selects it), on a way out
// goes to that place, on a device opens its console as on the canvas; Backspace (or "Eine Ebene
// höher") goes one level up, Shift+Backspace (or "Zurück") back (Alt and the arrows nudge on
// the plan, as on the canvas).  Dragging moves places on their plan and
// devices: in a room onto a spot, a desk or into a rack (the lowest free units), back into the
// tray (no spot); on a floor into another room; in a rack to other units or out of it (the
// tray on the right).  Each drag, and each edit of a place, is one undo step (doc/phys.rs); a
// refusal (a desktop device into a rack, units taken, the last room) puts everything back and
// says why above the plan.  The cables keep their ports and their automatic lengths follow.
//
// Selection: devices and cables share the canvas' selection (the inspector shows them, also
// what the logical view selected); a place is selected here alone (the inspector shows it when
// nothing else is).  The keyboard works as on the canvas: Tab walks the places, devices and
// ways out, then the cables; arrows nudge; Del removes the selected place (devices and cables
// as on the canvas); F2 renames a place or a device; screen readers get one record per place
// ("Raum Serverraum, 6 Geräte"), device ("SW1, Rack 1, HE 12") and cable ("Kabel PC1 eth0 –
// SW1 Gi1/0/1, 9 m, erlaubt 100 m (1000BASE-T)").

pub(crate) mod paint;
pub(crate) mod props;
pub(crate) mod scene;

use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::phys::{self, PhysErr, PlaceKind, Tree};
use jfn_model::Project;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::phys as dphys;
use crate::workspace::canvas::tools::Tool;
use scene::{inside, key_device, key_exit, key_place, place_key, Level, Scene, What, TOP};

/// the kinds of places in the order of the commands CMD_PHYS_ADD0 + k
pub(crate) const KINDS: [PlaceKind; 6] = [PlaceKind::Site, PlaceKind::Building, PlaceKind::Floor, PlaceKind::Room, PlaceKind::Rack, PlaceKind::Desk];
// i18n
const ADD_LABELS: [&str; 6] = ["Standort hinzufügen", "Gebäude hinzufügen", "Etage hinzufügen", "Raum hinzufügen", "Rack hinzufügen", "Tisch hinzufügen"];

/// a drag in progress: the project before it, the nodes it moves with where they started
struct Drag {
    before: dphys::Snap,
    start: Vec<(u64, [f32; 4])>,
}

/// the rename flyout: a place's name or a device's
pub(crate) struct Rename {
    place: Option<u32>,
    dev: Option<u32>,
    text: String,
    err: String,
    opened: bool,
    select: bool,
}

/// the edit buffers of the inspector's parts (props.rs)
#[derive(Default)]
pub(crate) struct Bufs {
    pub name: String,
    pub name_src: (u32, String),
    pub fields: [String; 6],
    pub fields_src: (u32, [i64; 6]),
    pub unit: String,
    pub unit_src: (u32, u16),
    pub len: String,
    pub len_src: (u32, u32),
    /// the last refusal and what it is about (another thing in the inspector clears it)
    pub err: String,
    pub err_for: u64,
}

impl Bufs {
    /// the inspector shows `key` now: a refusal about something else goes
    pub(crate) fn scope(&mut self, key: u64) {
        if self.err_for != key {
            self.err_for = key;
            self.err.clear();
        }
    }
}

pub(crate) struct Phys {
    /// the physical view is shown (else the logical canvas)
    pub on: bool,
    /// the place shown (TOP: every site)
    pub at: u32,
    /// the path of the place shown when it last existed (a removed one goes to what is left)
    path: Vec<u32>,
    /// the places shown before (Zurück)
    back: Vec<u32>,
    pub st: DiagramState,
    pub scene: Scene,
    pub painter: Option<Rc<dyn DgPaint>>,
    /// the selected place
    pub sel_place: Option<u32>,
    drag: Option<Drag>,
    rename: Option<Rename>,
    /// why the last edit was refused (shown above the plan)
    pub err: String,
    fit: bool,
    pub bufs: Bufs,
}

impl Phys {
    pub(crate) fn new() -> Phys {
        Phys {
            on: false,
            at: TOP,
            path: Vec::new(),
            back: Vec::new(),
            st: DiagramState::new(),
            scene: Scene::default(),
            painter: None,
            sel_place: None,
            drag: None,
            rename: None,
            err: String::new(),
            fit: true,
            bufs: Bufs::default(),
        }
    }

    /// a loaded project: the view its layout names (the place 0: the default room)
    pub(crate) fn reset(&mut self, p: &Project) {
        let tree = Tree::of(p);
        self.on = p.layout.phys;
        self.at = match p.layout.phys_at {
            0 => match tree.default_room() {
                0 => TOP,
                r => r,
            },
            u32::MAX => TOP,
            x if tree.get(x).is_some() => x,
            _ => TOP,
        };
        self.path = tree.path(self.at);
        self.back.clear();
        self.sel_place = None;
        self.drag = None;
        self.rename = None;
        self.err.clear();
        self.fit = true;
        self.bufs = Bufs::default();
        self.scene.built = (0, 0);
    }

    /// the view into the project's layout (before saving)
    pub(crate) fn store_view(&self, p: &mut Project) {
        p.layout.phys = self.on;
        let tree = Tree::of(p);
        p.layout.phys_at = if self.at == TOP {
            u32::MAX
        } else if self.at == tree.default_room() {
            0
        } else {
            self.at
        };
    }
}

/// the zoom the toolbar and the status bar show: the shown view's
pub(crate) fn zoom_of(n: &Net) -> f32 {
    if n.phys.on { n.phys.st.vp.zoom } else { n.canvas.st.vp.zoom }
}

/// the switch of the toolbar: Logisch | Physisch
pub(crate) fn toolbar(n: &mut Net, ui: &mut Ui) {
    ui.next_size(1, 20);
    ui.separator();
    let mut sel = n.phys.on as i32;
    ui.next_flags(ST_COMPACT);
    if ui.segmented(ID_TB_VIEW, &mut sel, &[t("Logisch"), t("Physisch")]) & EV_ACT != 0 {
        set_view(n, sel == 1);
        ui.repass();
    }
    ui.tooltip(t("Ansicht: logisch (Strg+Umschalt+L) oder physisch (Strg+Umschalt+P)"));
}

fn set_view(n: &mut Net, on: bool) {
    if n.phys.on != on {
        n.phys.on = on;
        n.phys.fit = true;
        n.phys.rename = None;
        n.phys.err.clear();
    }
}

/// go to a place (TOP: every site); the place shown before goes on the way back
fn go(n: &mut Net, at: u32) {
    if n.phys.at == at {
        return;
    }
    n.phys.back.push(n.phys.at);
    if n.phys.back.len() > 64 {
        n.phys.back.remove(0);
    }
    n.phys.at = at;
    n.phys.fit = true;
    n.phys.err.clear();
    n.phys.st.sel.clear();
    n.phys.sel_place = None;
}

/// up one level (from a site: every site)
fn up(n: &mut Net) {
    let tree = Tree::of(&n.doc.proj);
    let (_, shown) = scene::level_of(&tree, n.phys.at);
    let parent = tree.get(shown).map_or(TOP, |x| if x.parent == 0 { TOP } else { x.parent });
    if shown != TOP {
        let came = shown;
        go(n, parent);
        // the place it came from stays selected
        n.phys.sel_place = Some(came);
    }
}

fn back(n: &mut Net) {
    if let Some(b) = n.phys.back.pop() {
        n.phys.at = b;
        n.phys.fit = true;
        n.phys.err.clear();
        n.phys.sel_place = None;
    }
}

/// the plan of the place a device stands in (a rack's front, a room), the device selected
pub(crate) fn show_device(n: &mut Net, id: u32) {
    let tree = Tree::of(&n.doc.proj);
    let Some(d) = n.doc.proj.device(id) else { return };
    let w = tree.whereis(d);
    let at = if w.rack != 0 { w.rack } else { w.room };
    n.phys.on = true;
    if at != 0 {
        go(n, at);
    }
    n.canvas.select_only(id);
}

/// the parent a new place of a kind gets in the view shown (None: it does not go here)
fn add_parent(n: &Net, tree: &Tree, kind: PlaceKind) -> Option<u32> {
    let (_, shown) = scene::level_of(tree, n.phys.at);
    match (kind.parent_kind(), tree.get(shown)) {
        (None, _) if shown == TOP => Some(0),
        (Some(k), Some(x)) if x.kind == k => Some(shown),
        _ => None,
    }
}

fn refuse(n: &mut Net, e: PhysErr) {
    n.phys.err = crate::app::tr::msg(&e.msg());
}

fn add(n: &mut Net, kind: PlaceKind) {
    let tree = Tree::of(&n.doc.proj);
    let Some(parent) = add_parent(n, &tree, kind) else { return };
    let mut made = 0;
    let names = dphys::names();
    match dphys::step(&mut n.doc, "Ort hinzufügen", |p| phys::add_place(p, kind, parent, t(kind.title()), &names).map(|id| made = id)) {
        Ok(_) => {
            n.phys.err.clear();
            n.phys.sel_place = Some(made);
            n.canvas.st.sel.clear();
        }
        Err(e) => refuse(n, e),
    }
}

fn remove(n: &mut Net, id: u32) {
    let names = dphys::names();
    match dphys::step(&mut n.doc, "Ort entfernen", |p| phys::remove_place(p, id, &names)) {
        Ok(_) => {
            n.phys.err.clear();
            n.phys.sel_place = None;
        }
        Err(e) => refuse(n, e),
    }
}

/// Del: the selected place goes; selected devices and cables go as on the canvas
fn delete_selection(n: &mut Net, ui: &mut Ui) {
    if let Some(id) = n.phys.sel_place {
        remove(n, id);
    } else if !n.canvas.st.sel.is_empty() {
        crate::workspace::canvas::delete_selection(n, ui);
    }
}

fn open_rename(n: &mut Net) {
    let tree = Tree::of(&n.doc.proj);
    if let Some(id) = n.phys.sel_place {
        let text = scene::place_name(&tree, id);
        n.phys.rename = Some(Rename { place: Some(id), dev: None, text, err: String::new(), opened: false, select: true });
    } else if let Some(id) = crate::workspace::canvas::single(n).filter(|&d| n.doc.proj.device(d).is_some()) {
        let text = n.doc.proj.device(id).map(|d| d.name.clone()).unwrap_or_default();
        n.phys.rename = Some(Rename { place: None, dev: Some(id), text, err: String::new(), opened: false, select: true });
    }
}

fn zoom(n: &mut Net, k: i32) {
    let vp = &mut n.phys.st.vp;
    let r = vp.rect;
    let (cx, cy) = (r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
    match k {
        1 | -1 => vp.step(k, cx, cy),
        0 => vp.zoom_at(1.0, cx, cy),
        _ => n.phys.st.request_fit(),
    }
}

/// the view's commands (None: not one of them, or the view is not shown)
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, _arg: u32) -> Option<bool> {
    match id {
        CMD_VIEW_LOGICAL => set_view(n, false),
        CMD_VIEW_PHYS => set_view(n, true),
        _ if !n.phys.on => return None,
        CMD_ZOOM_IN => zoom(n, 1),
        CMD_ZOOM_OUT => zoom(n, -1),
        CMD_ZOOM_100 => zoom(n, 0),
        CMD_FIT => zoom(n, 2),
        CMD_FOCUS_CANVAS => ui.focus(ID_PH_CANVAS),
        CMD_DELETE => delete_selection(n, ui),
        CMD_RENAME | CMD_PHYS_RENAME => open_rename(n),
        CMD_SELECT_ALL => {
            let keys: Vec<u64> = n.phys.scene.items.iter().filter(|it| it.what == What::Device).map(|it| it.key).collect();
            n.canvas.st.sel = keys.into_iter().collect();
            n.phys.sel_place = None;
        }
        CMD_PHYS_UP => up(n),
        CMD_PHYS_BACK => back(n),
        CMD_PHYS_OPEN => {
            if let Some(p) = n.phys.sel_place {
                open(n, p);
            }
        }
        CMD_PHYS_REMOVE => {
            if let Some(p) = n.phys.sel_place {
                remove(n, p);
            }
        }
        _ if (CMD_PHYS_ADD0..CMD_PHYS_ADD0 + KINDS.len() as u32).contains(&id) => add(n, KINDS[(id - CMD_PHYS_ADD0) as usize]),
        _ => return None,
    }
    ui.repass();
    Some(true)
}

/// enabled and checked of the view's commands (after the canvas' own: they win while shown)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let on = n.phys.on;
    let flag = |c: bool, f: u32| if c { f } else { 0 };
    ui.cmd_state(CMD_VIEW_LOGICAL, flag(!on, CF_CHECKED));
    ui.cmd_state(CMD_VIEW_PHYS, flag(on, CF_CHECKED));
    let tree = Tree::of(&n.doc.proj);
    let place = n.phys.sel_place.filter(|&p| tree.get(p).is_some());
    ui.cmd_state(CMD_PHYS_UP, flag(!on || n.phys.at == TOP, CF_DISABLED));
    ui.cmd_state(CMD_PHYS_BACK, flag(!on || n.phys.back.is_empty(), CF_DISABLED));
    ui.cmd_state(CMD_PHYS_OPEN, flag(!on || place.is_none(), CF_DISABLED));
    ui.cmd_state(CMD_PHYS_RENAME, flag(!on || place.is_none(), CF_DISABLED));
    ui.cmd_state(CMD_PHYS_REMOVE, flag(!on || place.is_none(), CF_DISABLED));
    for (k, &kind) in KINDS.iter().enumerate() {
        ui.cmd_state(CMD_PHYS_ADD0 + k as u32, flag(!on || add_parent(n, &tree, kind).is_none(), CF_DISABLED));
    }
    if on {
        let dev = crate::workspace::canvas::single(n).filter(|&d| n.doc.proj.device(d).is_some());
        ui.cmd_state(CMD_RENAME, flag(place.is_none() && dev.is_none(), CF_DISABLED));
        ui.cmd_state(CMD_DELETE, flag(place.is_none() && n.canvas.st.sel.is_empty(), CF_DISABLED));
    }
}

/// open a place: a desk is selected (its room shows), any other place shows
fn open(n: &mut Net, id: u32) {
    let tree = Tree::of(&n.doc.proj);
    match tree.get(id).map(|x| (x.kind, x.parent)) {
        Some((PlaceKind::Desk, parent)) => {
            if n.phys.at != parent {
                go(n, parent);
            }
            n.phys.sel_place = Some(id);
        }
        Some(_) => go(n, id),
        None => {}
    }
}

/// the view in the dock's centre
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    let tree = Tree::of(&n.doc.proj);
    // the place shown went (undo, removed): what is left of its path
    if n.phys.at != TOP && tree.get(n.phys.at).is_none() {
        n.phys.at = n.phys.path.iter().rev().copied().find(|&i| tree.get(i).is_some()).unwrap_or(TOP);
        n.phys.fit = true;
    }
    n.phys.path = tree.path(n.phys.at);
    if n.phys.sel_place.is_some_and(|p| tree.get(p).is_none()) {
        n.phys.sel_place = None;
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    bar(n, ui, &tree);
    // the scene follows the project and the place shown
    if n.phys.scene.built != (n.doc.rev, n.phys.at) || n.phys.scene.ver == 0 {
        n.phys.scene.build(&n.doc.proj, &tree, &n.syms, n.phys.at, n.doc.rev);
    }
    let level = n.phys.scene.level;
    let st = &mut n.phys.st;
    st.grid = match level {
        Level::Top => 100.0,
        Level::Site => 40.0,
        Level::Building => 20.0,
        Level::Floor => 20.0,
        Level::Room => 20.0,
        Level::Rack => scene::RU,
    };
    st.snap = n.prefs.snap;
    st.tab_edges = true;
    if st.name != t("Physische Ansicht") {
        st.name = String::from(t("Physische Ansicht"));
    }
    // the selection: devices and cables are the canvas', a place is this view's
    if !st.busy() {
        let mut want: alloc::collections::BTreeSet<u64> = n.canvas.st.sel.iter().copied().collect();
        if let Some(p) = n.phys.sel_place {
            want.insert(place_key(p));
        }
        if st.sel != want {
            st.sel = want;
        }
    }
    // a new place shows whole: its outline, the tray, the places and devices in it (the canvas
    // keeps its size from the last pass).  The first time the widget fits its nodes itself and
    // the canvas has no size yet: the fit follows after it, with one more pass
    let first = n.phys.st.vp.rect[2] == 0;
    if n.phys.fit && !first && n.phys.st.vp.rect[3] > 0 {
        n.phys.fit = false;
        let b = n.phys.scene.extent();
        n.phys.st.vp.fit(b, 32);
    }
    let tool = if n.canvas.tool == Tool::Hand { DG_PAN } else { DG_SELECT };
    let ver = n.syms.key() ^ n.phys.scene.ver.rotate_left(9) ^ if crate::app::tr::en() { 0x4000_0000 } else { 0 };
    ui.next_size(-1, -1);
    let evs = {
        let ph = &mut n.phys;
        let m = ph.scene.model();
        let evs = ui.diagram(ID_PH_CANVAS, &mut ph.st, &m, tool, ph.painter.clone(), ver);
        if let DHit::Edge { key, .. } = ph.st.hover {
            let text = ph.scene.edge_label(key);
            ui.tooltip_part(&text, key);
        } else if let DHit::Node(key) = ph.st.hover {
            let text = ph.scene.label(key);
            ui.tooltip_part(&text, key);
        }
        let sc = &ph.scene;
        ui.diagram_sems(ID_PH_CANVAS, &mut ph.st, &m, &|k| sc.label(k));
        ui.diagram_edge_sems(ID_PH_CANVAS, &mut ph.st, &m, &|k| sc.edge_label(k));
        evs
    };
    ui.box_end();
    // the canvas got its size only now (the first time): it fits, one more pass shows it
    if n.phys.fit && n.phys.st.vp.rect[2] > 0 && n.phys.st.vp.rect[3] > 0 {
        n.phys.fit = false;
        let b = n.phys.scene.extent();
        n.phys.st.vp.fit(b, 32);
        ui.repass();
    }
    for ev in evs {
        event(n, ui, ev);
    }
    rename_flyout(n, ui);
}

// the bar above the plan: back, up, the breadcrumbs, renaming and removing the selected place;
// below it the places to add here and the refusal or a hint
fn bar(n: &mut Net, ui: &mut Ui, tree: &Tree) {
    ui.box_begin(0, BOX_H | BOX_XCENTER | pad(6), 4);
    ui.next_flags(if n.phys.back.is_empty() { ST_DISABLED } else { 0 });
    if ui.icon_button(ID_PH_BACK, IC_BACK, t("Zurück")) & EV_ACT != 0 {
        back(n);
        ui.repass();
    }
    ui.tooltip(t("Zurück (Umschalt+Rücktaste)"));
    ui.next_flags(if n.phys.at == TOP { ST_DISABLED } else { 0 });
    if ui.icon_button(ID_PH_UP, IC_UP, t("Eine Ebene höher")) & EV_ACT != 0 {
        up(n);
        ui.repass();
    }
    ui.tooltip(t("Eine Ebene höher (Rücktaste)"));
    let (_, shown) = scene::level_of(tree, n.phys.at);
    let mut ids: Vec<u32> = alloc::vec![TOP];
    if shown != TOP {
        ids.extend(tree.path(shown));
    }
    let names: Vec<String> = ids.iter().map(|&i| if i == TOP { String::from(t("Alle Standorte")) } else { scene::place_name(tree, i) }).collect();
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    if let Some(i) = ui.breadcrumb(ID_PH_CRUMB, &refs) {
        go(n, ids[i]);
        ui.repass();
    }
    ui.spacer(-1);
    let place = n.phys.sel_place.filter(|&p| tree.get(p).is_some());
    ui.next_flags(if place.is_none() { ST_DISABLED } else { 0 });
    if ui.icon_button(ID_PH_RENAME, IC_EDIT, t("Ort umbenennen")) & EV_ACT != 0 {
        open_rename(n);
        ui.repass();
    }
    ui.tooltip(t("Ort umbenennen (F2)"));
    ui.next_flags(if place.is_none() { ST_DISABLED } else { 0 });
    if ui.icon_button(ID_PH_REMOVE, IC_DELETE, t("Ort entfernen")) & EV_ACT != 0 {
        if let Some(p) = place {
            remove(n, p);
        }
        ui.repass();
    }
    ui.tooltip(t("Ort entfernen (Entf): seine Geräte kommen in den ersten Raum"));
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER | pad(6), 6);
    for (k, &kind) in KINDS.iter().enumerate() {
        if add_parent(n, tree, kind).is_some() {
            ui.next_icon(IC_ADD);
            ui.next_flags(ST_COMPACT);
            if ui.button(ID_PH_ADD0 + k as u32, t(ADD_LABELS[k])) & EV_ACT != 0 {
                add(n, kind);
                ui.repass();
            }
        }
    }
    if !n.phys.err.is_empty() {
        let e = n.phys.err.clone();
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 0);
        crate::workspace::error_text(ui, &e, 0);
        ui.box_end();
    } else {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, t(HINTS[n.phys.scene.level as usize]));
    }
    ui.box_end();
}

// i18n
const HINTS: [&str; 6] = [
    "Ziehen verschiebt Standorte; ein Doppelklick öffnet einen Standort.",
    "Ziehen verschiebt Gebäude; ein Doppelklick öffnet ein Gebäude.",
    "Die Etagen übereinander; ein Doppelklick öffnet eine Etage.",
    "Ziehen verschiebt Räume und Geräte in einen anderen Raum; ein Doppelklick öffnet einen Raum.",
    "Ziehen stellt Geräte an einen Platz, auf einen Tisch, in ein Rack oder zurück in die Ablage; ein Doppelklick öffnet ein Rack.",
    "Ziehen steckt Geräte in andere Höheneinheiten, nach rechts aus dem Rack.",
];

fn event(n: &mut Net, ui: &mut Ui, ev: DEv) {
    match ev {
        DEv::Select => {
            let st = &n.phys.st;
            n.canvas.st.sel = st.sel.iter().copied().filter(|&k| key_device(k).is_some()).collect();
            n.phys.sel_place = st.sel.iter().find_map(|&k| key_place(k));
            n.phys.err.clear();
        }
        DEv::Move { keys, total, done, .. } => drag(n, &keys, total, done),
        DEv::Activate(DHit::Node(k)) => {
            if let Some(p) = key_place(k) {
                open(n, p);
            } else if let Some(p) = key_exit(k) {
                open(n, p);
            } else if let Some(d) = key_device(k).filter(|&d| n.doc.proj.device(d).is_some()) {
                crate::workspace::terminal::activate(n, ui, d);
            }
            ui.repass();
        }
        DEv::Activate(DHit::Edge { .. }) => {
            n.shell.dock.focus(P_PROPS);
            ui.repass();
        }
        DEv::Context(hit, _) => {
            let menu: &'static [Mi] = match hit {
                DHit::Node(k) if key_place(k).is_some() => {
                    n.phys.sel_place = key_place(k);
                    n.canvas.st.sel.clear();
                    &crate::app::commands::M_PHYS_PLACE
                }
                DHit::Node(k) if key_device(k).is_some_and(|d| n.doc.proj.device(d).is_some()) => &crate::app::commands::M_DEVICE,
                _ => &crate::app::commands::M_PHYS_BG,
            };
            ui.menu_popup(crate::app::tr::menu(menu));
        }
        DEv::Delete => delete_selection(n, ui),
        _ => {}
    }
}

fn rnd(v: f32) -> i32 {
    let f = v + 0.5;
    let t = f as i32;
    if (t as f32) > f { t - 1 } else { t }
}

// a drag (or an arrow key) of places and devices: live in the project, one undo step at its end
fn drag(n: &mut Net, keys: &[u64], total: [f32; 2], done: bool) {
    if n.phys.drag.is_none() {
        let start: Vec<(u64, [f32; 4])> = keys.iter().filter_map(|&k| n.phys.scene.item(k).filter(|it| !matches!(it.what, What::Exit(_))).map(|it| (k, it.rect))).collect();
        n.phys.drag = Some(Drag { before: dphys::snapshot(&n.doc.proj), start });
    }
    let Some(d) = n.phys.drag.take() else { return };
    let sc = &n.phys.scene;
    let (level, k, at) = (sc.level, sc.k.max(1e-6), sc.at);
    let tree = Tree::of(&n.doc.proj);
    let units = sc.rack.map_or(0, |r| r.units);
    let moved: Vec<(u64, [f32; 4])> = d.start.iter().map(|&(key, r)| (key, [r[0] + total[0], r[1] + total[1], r[2], r[3]])).collect();
    dphys::live(&mut n.doc, |p| {
        // the default tree is stored first: its ids stay what the plan showed
        let names = dphys::names();
        phys::ensure(p, &names);
        for &(key, r) in &moved {
            let c = [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
            if let Some(id) = key_place(key) {
                if let Some(x) = p.places.iter_mut().find(|x| x.id == id) {
                    x.x_dm = rnd(r[0] / k);
                    x.y_dm = rnd(r[1] / k);
                }
                continue;
            }
            let Some(dev) = key_device(key) else { continue };
            let Some(dv) = p.device(dev) else { continue };
            let w = tree.whereis(dv);
            let h = p.model_of(dev).map_or(1, |m| jfn_model::catalog::rack_units(m).max(1)) as i32;
            let spot = match level {
                Level::Room => Some(jfn_model::Spot { place: at, x_dm: rnd(c[0] / k), y_dm: rnd(c[1] / k), placed: true, unit: 0 }),
                Level::Floor => {
                    let o = tree.get(w.room).map_or((0, 0), |x| (x.x_dm, x.y_dm));
                    Some(jfn_model::Spot { place: w.room, x_dm: rnd(c[0] / k) - o.0, y_dm: rnd(c[1] / k) - o.1, placed: true, unit: 0 })
                }
                Level::Rack => {
                    let top = units as i32 - rnd((r[1] - scene::RACK_PAD) / scene::RU);
                    let unit = (top - h + 1).clamp(1, (units as i32 - h + 1).max(1));
                    Some(jfn_model::Spot { place: at, unit: unit as u16, ..jfn_model::Spot::default() })
                }
                _ => None,
            };
            if let (Some(s), Some(dm)) = (spot, p.device_mut(dev)) {
                dm.phys = s;
            }
        }
    });
    if !done {
        n.phys.drag = Some(d);
        return;
    }
    // the end: where the devices landed decides their place
    if total == [0.0, 0.0] {
        dphys::restore(&mut n.doc, &d.before);
        return;
    }
    let names = dphys::names();
    let mut res: Result<(), PhysErr> = Ok(());
    for &(key, r) in &moved {
        let Some(dev) = key_device(key) else { continue };
        let c = [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5];
        let sc = &n.phys.scene;
        let p = &mut n.doc.proj;
        let room = Tree::of(p).up(at, PlaceKind::Room).unwrap_or(at);
        res = match level {
            Level::Room => {
                if let Some(rack) = sc.place_at(c, &[PlaceKind::Rack]) {
                    phys::place_device(p, dev, rack, 0, None, &names)
                } else if let Some(desk) = sc.place_at(c, &[PlaceKind::Desk]) {
                    phys::place_device(p, dev, desk, 0, None, &names)
                } else if sc.frame.is_some_and(|f| inside(f, c)) {
                    phys::place_device(p, dev, at, 0, Some((rnd(c[0] / k), rnd(c[1] / k))), &names)
                } else {
                    phys::place_device(p, dev, at, 0, None, &names)
                }
            }
            Level::Floor => {
                if let Some(x) = sc.place_at(c, &[PlaceKind::Rack, PlaceKind::Desk]) {
                    phys::place_device(p, dev, x, 0, None, &names)
                } else if let Some(r2) = sc.place_at(c, &[PlaceKind::Room]) {
                    let o = Tree::of(p).get(r2).map_or((0, 0), |x| (x.x_dm, x.y_dm));
                    phys::place_device(p, dev, r2, 0, Some((rnd(c[0] / k) - o.0, rnd(c[1] / k) - o.1)), &names)
                } else {
                    Err(PhysErr::NoDevices(PlaceKind::Floor))
                }
            }
            Level::Rack if c[0] >= scene::RACK_TRAY_X - 20.0 => phys::place_device(p, dev, room, 0, None, &names),
            Level::Rack => {
                let unit = p.device(dev).map_or(0, |d| d.phys.unit);
                phys::place_device(p, dev, at, unit, None, &names)
            }
            _ => Ok(()),
        };
        if res.is_err() {
            break;
        }
    }
    match res {
        Ok(()) => {
            n.phys.err.clear();
            let label = if moved.iter().any(|m| key_place(m.0).is_some()) { "Ort verschieben" } else { "Gerät umstellen" };
            dphys::since(&mut n.doc, d.before, label);
        }
        Err(e) => {
            dphys::restore(&mut n.doc, &d.before);
            refuse(n, e);
        }
    }
}

// the flyout under the place or device being renamed
fn rename_flyout(n: &mut Net, ui: &mut Ui) {
    if n.phys.rename.is_none() && ui.popup_is_open(ID_PH_FLY) {
        ui.popup_close();
    }
    let mut gone = false;
    if let Some(r) = &mut n.phys.rename {
        if !r.opened {
            r.opened = true;
            ui.popup_open(ID_PH_FLY);
            ui.focus(ID_PH_FLY_BOX);
        } else if !ui.popup_is_open(ID_PH_FLY) {
            gone = true;
        }
    }
    if gone {
        n.phys.rename = None;
    }
    let Some(key) = n.phys.rename.as_ref().map(|r| r.place.map_or_else(|| r.dev.unwrap_or(0) as u64, place_key)) else { return };
    // the anchor: the node on the screen (else the plan's top left)
    let vp = n.phys.st.vp;
    let rect = n.phys.scene.item(key).map_or([vp.to_world(vp.rect[0] as f32 + 20.0, vp.rect[1] as f32 + 20.0)[0], vp.to_world(vp.rect[0] as f32, vp.rect[1] as f32 + 20.0)[1], 1.0, 1.0], |it| it.rect);
    let q = vp.rect_to_screen(rect);
    let c = vp.rect;
    let x0 = (q[0] as i32).clamp(c[0], c[0] + c[2] - 1);
    let x1 = ((q[0] + q[2]) as i32).clamp(x0 + 1, c[0] + c[2]);
    let y1 = ((q[1] + q[3]) as i32).clamp(c[1] + 1, c[1] + c[3]);
    let y0 = (q[1] as i32).clamp(c[1], y1 - 1);
    ui.last_set([x0, y0, x1 - x0, y1 - y0], 0);
    let mut act = 0;
    if ui.flyout_begin(ID_PH_FLY, FL_BELOW) {
        if let Some(r) = &mut n.phys.rename {
            ui.box_begin(0, BOX_V | pad(8), 6);
            ui.text(TS_BODY_STRONG, t(if r.place.is_some() { "Name des Orts" } else { "Gerätename" }));
            if ui.focused() == ID_PH_FLY_BOX && ui.capture_shortcut(VK_ESCAPE, 0) {
                act = 2;
            }
            ui.next_size(240, 0);
            let ev = ui.textbox(ID_PH_FLY_BOX, Some(t("Name")), &mut r.text, 64);
            if r.select && ui.textbox_select(ID_PH_FLY_BOX, &r.text, 0, r.text.len()) {
                r.select = false;
            }
            if ev & EV_ACT != 0 {
                r.err.clear();
            }
            if ev & EV_COMMIT != 0 && ui.focused() == ID_PH_FLY_BOX {
                act = 1;
            }
            if !r.err.is_empty() {
                crate::workspace::error_text(ui, &r.err, 240);
            }
            ui.box_begin(0, BOX_H | BOX_END, 8);
            if ui.accent_button(ID_PH_FLY_OK, t("Umbenennen")) & EV_ACT != 0 {
                act = 1;
            }
            if ui.button(ID_PH_FLY_CANCEL, t("Abbrechen")) & EV_ACT != 0 {
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
            ui.popup_close();
            n.phys.rename = None;
        }
        _ => {}
    }
}

fn commit_rename(n: &mut Net, ui: &mut Ui) {
    let Some(r) = &n.phys.rename else { return };
    let text = r.text.clone();
    let res: Result<(), String> = match (r.place, r.dev) {
        (Some(id), _) => {
            let names = dphys::names();
            dphys::step(&mut n.doc, "Ort umbenennen", |p| phys::rename_place(p, id, &text, &names)).map(|_| ()).map_err(|e| crate::app::tr::msg(&e.msg()))
        }
        (None, Some(id)) => match crate::doc::edits::rename(&mut n.doc.proj, id, &text) {
            Ok(Some(e)) => {
                n.doc.record(e);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => Err(crate::app::tr::msg(&e.msg())),
        },
        _ => Ok(()),
    };
    match res {
        Ok(()) => {
            ui.popup_close();
            n.phys.rename = None;
        }
        Err(e) => {
            if let Some(r) = &mut n.phys.rename {
                r.err = e;
            }
        }
    }
    ui.repass();
}

/// the count of devices in a place, as words
pub(crate) fn count_text(k: usize) -> String {
    if k == 1 { String::from(t("1 Gerät")) } else { tf("{} Geräte", &[&k.to_string()]) }
}
