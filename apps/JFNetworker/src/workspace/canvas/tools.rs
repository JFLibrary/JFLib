// tools -- the tools of the topology canvas and what a click with them does
//
//   Tool::{Select, Hand, Connect, Note, Place(model)}   the toolbar's tools (V, H, C, N) and a
//                                   palette tile armed for placing (DG_PLACE); Pdu (P) picks the
//                                   source and destination of a simple PDU (DG_PLACE, sim/pdu.rs)
//   tool.dg() / status()            the ui::diagram tool, the status text
//   set(n, tool)                    switch (the armed tile is dropped)
//   arm(n, model)                   a tile was clicked: place it with the next click (again:
//                                   back to Auswahl)
//   place(n, model, at)             a device centred on the world point at (snapped), selected
//   place_note(n, at)               a note at the point, then its editor opens
//   place_centre(n, model)          Enter on a tile: into the middle of the view
//   origin(n, at) / click_point(n, at)   where a device lands (the canvas' preview)
//
// Connect (C) draws cables: from one device to another, or two clicks (connect.rs); it stays
// until Esc or another tool.  A placing click with Shift keeps the tile armed for the next
// device; without it the canvas returns to Auswahl.

use alloc::string::String;

use jfn_model::catalog;
use ui::*;

use super::paint::{DEV_H, DEV_W};
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    Select,
    Hand,
    Connect,
    Note,
    /// a catalog model (its index in catalog()) waits for a click on the canvas
    Place(usize),
    /// simple PDUs (P): a click on the source, then on the destination (workspace/sim/pdu.rs)
    Pdu,
}

impl Tool {
    /// the ui::diagram tool
    pub(crate) fn dg(self) -> u32 {
        match self {
            Tool::Select => DG_SELECT,
            Tool::Hand => DG_PAN,
            Tool::Connect => DG_CONNECT,
            Tool::Note | Tool::Place(_) | Tool::Pdu => DG_PLACE,
        }
    }

    /// the mode in the status bar
    pub(crate) fn status(self) -> String {
        match self {
            Tool::Select => String::from(t("Auswahl")),
            Tool::Hand => String::from(t("Hand")),
            Tool::Connect => String::from(t("Verbinden")),
            Tool::Note => String::from(t("Notiz")),
            Tool::Place(i) => tf("Platzieren: {}", &[catalog::catalog().get(i).map_or("?", |m| t(m.name))]),
            Tool::Pdu => String::from(t("Einfache PDU")),
        }
    }
}

/// switch the tool
pub(crate) fn set(n: &mut Net, tool: Tool) {
    n.canvas.tool = tool;
    n.canvas.conn = None;
    n.canvas.kb_from = None;
}

/// a palette tile was clicked: arm it (a second click disarms); the note tile is the Notiz tool
pub(crate) fn arm(n: &mut Net, model: usize) {
    let note = catalog::catalog().get(model).is_some_and(|m| m.has(catalog::F_ANNOTATION));
    let t = if note { Tool::Note } else { Tool::Place(model) };
    n.canvas.tool = if n.canvas.tool == t { Tool::Select } else { t };
}

// round to whole world units
fn rnd(v: f32) -> i32 {
    let f = v + 0.5;
    let t = f as i32;
    if (t as f32) > f { t - 1 } else { t }
}

// a top left corner snapped to the canvas grid when snapping is on
fn snapped(n: &Net, x: f32, y: f32) -> [i32; 2] {
    let st = &n.canvas.st;
    if st.snap && st.grid > 0.0 {
        let g = st.grid;
        [rnd(x / g) * g as i32, rnd(y / g) * g as i32]
    } else {
        [rnd(x), rnd(y)]
    }
}

/// the top left corner of a device centred on the world point at (snapped)
pub(crate) fn origin(n: &Net, at: [f32; 2]) -> [i32; 2] {
    snapped(n, at[0] - DEV_W * 0.5, at[1] - DEV_H * 0.5)
}

/// a world point snapped to the grid as the canvas snaps a placing click
pub(crate) fn click_point(n: &Net, at: [f32; 2]) -> [f32; 2] {
    let st = &n.canvas.st;
    if st.snap && st.grid > 0.0 {
        let g = st.grid;
        [rnd(at[0] / g) as f32 * g, rnd(at[1] / g) as f32 * g]
    } else {
        at
    }
}

/// a device of catalog model `model` centred on the world point at; selected afterwards
pub(crate) fn place(n: &mut Net, ui: &mut Ui, model: usize, at: [f32; 2]) -> Option<u32> {
    let m = catalog::catalog().get(model)?;
    if m.has(catalog::F_ANNOTATION) {
        return place_note(n, ui, at);
    }
    let [x, y] = origin(n, at);
    match edits::add_device(&mut n.doc.proj, m.id, x, y) {
        Ok((id, e)) => {
            n.doc.record(e);
            n.canvas.select_only(id);
            ui.repass();
            Some(id)
        }
        Err(e) => {
            n.file.msg = Some((IB_ERROR, String::from(t("Das Gerät wurde nicht eingefügt")), crate::app::tr::msg(&e.msg())));
            None
        }
    }
}

/// Enter on a tile: the device into the middle of the view (moved right by its width while
/// another device lies there), selected
pub(crate) fn place_centre(n: &mut Net, ui: &mut Ui, model: usize) -> Option<u32> {
    let vp = n.canvas.st.vp;
    let c = [vp.cx as f32, vp.cy as f32];
    let m = catalog::catalog().get(model)?;
    if m.has(catalog::F_ANNOTATION) {
        return place_note(n, ui, c);
    }
    let g = if n.canvas.st.grid > 0.0 { n.canvas.st.grid } else { 20.0 };
    let step = ((DEV_W + 16.0) / g + 0.999) as i32 as f32 * g;
    let mut at = c;
    for _ in 0..40 {
        let [x, y] = origin(n, at);
        let taken = n.doc.proj.devices.iter().any(|d| (d.x - x).abs() < DEV_W as i32 && (d.y - y).abs() < DEV_H as i32);
        if !taken {
            break;
        }
        at[0] += step;
    }
    place(n, ui, model, at)
}

/// a note with its top left corner at the world point; its editor opens
pub(crate) fn place_note(n: &mut Net, ui: &mut Ui, at: [f32; 2]) -> Option<u32> {
    let [x, y] = snapped(n, at[0], at[1]);
    let (id, e) = edits::add_note(&mut n.doc.proj, x, y);
    n.doc.record(e);
    n.canvas.select_only(id);
    n.canvas.open_rename(&n.doc.proj, id);
    ui.repass();
    Some(id)
}
